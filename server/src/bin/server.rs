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

use auth::{AuthConfig, AuthService};
use data_service::PgDataService;
use evm::monitor::bridge::{COMMANDS_CHANNEL, EVENTS_CHANNEL, RedisBridge};
use rates::RateProviderConfig;
use server::{
    AppState, CleanupConfig, EventConsumer, InvoiceCleanupService, RedisEVMMonitor,
    WatchRetryConfig, WatchRetryService, WebhookConfig, WebhookService, api, config::Config,
    metrics,
};

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file if present
    let _ = dotenvy::dotenv();

    // Load configuration
    let config = Config::from_env()?;

    // Initialize tracing
    init_tracing(&config.log_level);

    // Initialize Prometheus metrics
    metrics::init_metrics()?;

    tracing::info!("Starting ETHPayServer v{}", env!("CARGO_PKG_VERSION"));

    // Connect to database
    tracing::info!("Connecting to database...");
    let data_service = Arc::new(PgDataService::connect(&config.database_url).await?);
    tracing::info!("Database connected");

    // Create auth service with custom config
    // Note: PgDataService implements AuthRepository (all required traits)
    let mut auth_config = AuthConfig {
        // Increase wallet challenge timeout to 10 minutes for registration flow
        wallet_challenge_duration: chrono::Duration::minutes(10),
        ..AuthConfig::default()
    };

    // WebAuthn config from environment
    let rp_id_explicit = std::env::var("WEBAUTHN_RP_ID").ok();
    if let Ok(rp_origin) = std::env::var("WEBAUTHN_RP_ORIGIN") {
        auth_config.rp_origin = rp_origin;
    }
    if let Ok(rp_name) = std::env::var("WEBAUTHN_RP_NAME") {
        auth_config.rp_name = rp_name;
    }

    // Set rp_id: use explicit env var, or auto-derive from rp_origin's host.
    // This prevents the common misconfiguration where WEBAUTHN_RP_ORIGIN is set
    // to the deployment URL but WEBAUTHN_RP_ID is left as the "localhost" default,
    // causing "rp.id cannot be used with the current origin" errors in the browser.
    if let Some(rp_id) = rp_id_explicit {
        auth_config.rp_id = rp_id;
    } else if let Ok(url) = url::Url::parse(&auth_config.rp_origin) {
        if let Some(host) = url.host_str() {
            auth_config.rp_id = host.to_string();
        }
    }

    // Validate WebAuthn config at startup to fail fast on misconfiguration
    let rp_origin_url = url::Url::parse(&auth_config.rp_origin)
        .expect("WEBAUTHN_RP_ORIGIN must be a valid URL (e.g. https://testnet.random.cash)");
    if let Some(host) = rp_origin_url.host_str() {
        if !host.ends_with(&auth_config.rp_id) {
            tracing::error!(
                rp_id = %auth_config.rp_id,
                origin_host = %host,
                "WEBAUTHN_RP_ID must be a registrable domain suffix of WEBAUTHN_RP_ORIGIN host — passkey auth will fail"
            );
        }
    }

    tracing::info!(rp_id = %auth_config.rp_id, rp_origin = %auth_config.rp_origin, "WebAuthn configured");
    let auth_service = Arc::new(AuthService::with_config(
        Arc::clone(&data_service),
        auth_config,
    ));

    // Connect to Redis (REQUIRED for event processing)
    let redis_url = config
        .redis_url
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("REDIS_URL is required for event processing"))?;

    tracing::info!("Connecting to Redis...");
    let events_channel =
        std::env::var("REDIS_EVENTS_CHANNEL").unwrap_or_else(|_| EVENTS_CHANNEL.to_string());
    let commands_channel =
        std::env::var("REDIS_COMMANDS_CHANNEL").unwrap_or_else(|_| COMMANDS_CHANNEL.to_string());
    tracing::debug!(
        events_channel,
        commands_channel,
        "Redis channels configured"
    );
    let bridge = Arc::new(RedisBridge::new(redis_url, &events_channel, &commands_channel).await?);
    tracing::info!("Redis connected");

    // Create EVM monitor using shared bridge (concrete type for generics)
    let evm_monitor = Arc::new(RedisEVMMonitor::new(Arc::clone(&bridge)));

    // Create WebSocket broadcast channel (shared by services and HTTP handler)
    let ws_broadcast = Arc::new(server::api::ws::WsBroadcast::new(256));

    // Start background services
    // 1. Webhook delivery service - sends webhook notifications
    //    Created first because cleanup service needs it for expiration webhooks
    let webhook_config = WebhookConfig::from_env();
    tracing::debug!(?webhook_config, "Webhook config loaded");
    let webhook_service = Arc::new(WebhookService::new(
        Arc::clone(&data_service),
        redis_url,
        webhook_config,
    )?);
    tokio::spawn(Arc::clone(&webhook_service).run());
    tracing::info!("Webhook delivery service started");

    // 2. Invoice cleanup service - expires invoices and unwatches addresses
    //    Also queues webhook notifications when invoices expire
    let cleanup_config = CleanupConfig::from_env();
    tracing::debug!(?cleanup_config, "Cleanup config loaded");
    let cleanup_service = Arc::new(InvoiceCleanupService::new(
        Arc::clone(&data_service),
        Arc::clone(&evm_monitor),
        cleanup_config,
        Some(Arc::clone(&webhook_service)),
        Some(Arc::clone(&ws_broadcast)),
    ));
    tokio::spawn(Arc::clone(&cleanup_service).run());
    tracing::info!("Invoice cleanup service started");

    // 3. Event consumer - processes PaymentDetected/Confirmed events
    //    Also triggers expiration checks on block events and queues webhooks
    let bridge_dyn: Arc<dyn evm::monitor::bridge::EventBridge> = bridge.clone();
    let event_consumer = EventConsumer::new(
        bridge_dyn,
        Arc::clone(&data_service),
        Some(cleanup_service),
        Some(webhook_service),
        Some(Arc::clone(&ws_broadcast)),
    );
    tokio::spawn(event_consumer.run());
    tracing::info!("Event consumer started");

    // 4. Watch retry service - retries failed WatchAddress commands
    let retry_config = WatchRetryConfig::from_env();
    tracing::debug!(?retry_config, "Watch retry config loaded");
    if retry_config.enabled {
        let retry_service = WatchRetryService::new(
            Arc::clone(&data_service),
            Arc::clone(&evm_monitor),
            retry_config,
        );
        tokio::spawn(retry_service.run());
        tracing::info!("Watch retry service started");
    } else {
        tracing::info!("Watch retry service disabled");
    }

    // Create rate provider
    let rate_config = RateProviderConfig::from_env();
    tracing::info!(provider = rate_config.provider, "Rate provider configured");
    let rate_provider = rate_config.create_provider();

    // Create application state
    let mut state = AppState::new(
        Arc::clone(&data_service),
        auth_service,
        Some(evm_monitor),
        rate_provider,
    );
    state.ws_broadcast = Some(ws_broadcast);

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
