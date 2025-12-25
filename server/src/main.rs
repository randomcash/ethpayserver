//! ETHPayServer main binary.
//!
//! Start the server with:
//! ```bash
//! DATABASE_URL="postgres://..." cargo run --release
//! ```

use std::sync::Arc;

use anyhow::Result;
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use auth::AuthService;
use data_service::PgDataService;
use server::{
    api, config::Config, AppState, CleanupConfig, EventConsumer,
    InvoiceCleanupService, RedisEVMMonitor, WatchRetryConfig, WatchRetryService,
    WebhookConfig, WebhookService,
};
use evm::monitor::bridge::{RedisBridge, COMMANDS_CHANNEL, EVENTS_CHANNEL};

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file if present
    let _ = dotenvy::dotenv();

    // Load configuration
    let config = Config::from_env()?;

    // Initialize tracing
    init_tracing(&config.log_level);

    tracing::info!("Starting ETHPayServer v{}", env!("CARGO_PKG_VERSION"));

    // Connect to database
    tracing::info!("Connecting to database...");
    let data_service = Arc::new(PgDataService::connect(&config.database_url).await?);
    tracing::info!("Database connected");

    // Create auth service
    // Note: PgDataService implements AuthRepository (all required traits)
    let auth_service = Arc::new(AuthService::new(Arc::clone(&data_service)));

    // Connect to Redis (REQUIRED for event processing)
    let redis_url = config.redis_url.as_ref()
        .ok_or_else(|| anyhow::anyhow!("REDIS_URL is required for event processing"))?;

    tracing::info!("Connecting to Redis...");
    let events_channel = std::env::var("REDIS_EVENTS_CHANNEL")
        .unwrap_or_else(|_| EVENTS_CHANNEL.to_string());
    let commands_channel = std::env::var("REDIS_COMMANDS_CHANNEL")
        .unwrap_or_else(|_| COMMANDS_CHANNEL.to_string());
    tracing::debug!(events_channel, commands_channel, "Redis channels configured");
    let bridge = Arc::new(
        RedisBridge::new(redis_url, &events_channel, &commands_channel).await?
    );
    tracing::info!("Redis connected");

    // Create EVM monitor using shared bridge (concrete type for generics)
    let evm_monitor = Arc::new(RedisEVMMonitor::new(Arc::clone(&bridge)));

    // Start background services
    // 1. Invoice cleanup service - expires invoices and unwatches addresses
    let cleanup_config = CleanupConfig::from_env();
    tracing::debug!(?cleanup_config, "Cleanup config loaded");
    let cleanup_service = Arc::new(InvoiceCleanupService::new(
        Arc::clone(&data_service),
        Arc::clone(&evm_monitor),
        cleanup_config,
    ));
    tokio::spawn(Arc::clone(&cleanup_service).run());
    tracing::info!("Invoice cleanup service started");

    // 2. Webhook delivery service - sends webhook notifications
    let webhook_config = WebhookConfig::from_env();
    tracing::debug!(?webhook_config, "Webhook config loaded");
    let webhook_service = Arc::new(
        WebhookService::new(
            Arc::clone(&data_service),
            redis_url,
            webhook_config,
        )?
    );
    tokio::spawn(Arc::clone(&webhook_service).run());
    tracing::info!("Webhook delivery service started");

    // 3. Event consumer - processes PaymentDetected/Confirmed events
    //    Also triggers expiration checks on block events and queues webhooks
    let bridge_dyn: Arc<dyn evm::monitor::bridge::EventBridge> = bridge.clone();
    let event_consumer = EventConsumer::new(
        bridge_dyn,
        Arc::clone(&data_service),
        Some(cleanup_service),
        Some(webhook_service),
    );
    tokio::spawn(event_consumer.run());
    tracing::info!("Event consumer started");

    // 4. Watch retry service - retries failed WatchAddress commands
    let evm_monitor_dyn: Arc<dyn server::EVMMonitor> = evm_monitor.clone();
    let retry_config = WatchRetryConfig::from_env();
    tracing::debug!(?retry_config, "Watch retry config loaded");
    if retry_config.enabled {
        let retry_service = WatchRetryService::new(
            Arc::clone(&data_service),
            evm_monitor_dyn.clone(),
            retry_config,
        );
        tokio::spawn(retry_service.run());
        tracing::info!("Watch retry service started");
    } else {
        tracing::info!("Watch retry service disabled");
    }

    // Wrap in Option for AppState compatibility
    let evm_monitor: Option<Arc<dyn server::EVMMonitor>> = Some(evm_monitor_dyn);

    // Create application state
    let state = AppState::new(Arc::clone(&data_service), auth_service, evm_monitor);

    // Build router with middleware
    let app = api::router(state, config.enable_swagger)
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );

    // Start server
    let bind_addr = config.bind_address();
    tracing::info!("Server listening on http://{}", bind_addr);

    if config.enable_swagger {
        tracing::info!("Swagger UI available at http://{}/swagger-ui", bind_addr);
    }

    let listener = TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn init_tracing(log_level: &str) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level));

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();
}
