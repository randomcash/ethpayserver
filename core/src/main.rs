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
use core::{api, config::Config, AppState, RedisEVMMonitor, WatchRetryConfig, WatchRetryService};
use data_service::PgDataService;

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

    // Connect to Redis for EVM monitor communication (optional)
    let evm_monitor: Option<Arc<dyn core::EVMMonitor>> = if let Some(ref redis_url) = config.redis_url {
        tracing::info!("Connecting to Redis for EVM monitor...");
        match RedisEVMMonitor::connect(redis_url).await {
            Ok(monitor) => {
                tracing::info!("Redis connected for EVM monitor");
                Some(Arc::new(monitor))
            }
            Err(e) => {
                tracing::warn!("Failed to connect to Redis: {}. EVM monitor disabled.", e);
                None
            }
        }
    } else {
        tracing::info!("No REDIS_URL configured, EVM monitor disabled");
        None
    };

    // Start background watch retry service if EVM monitor is configured
    if let Some(ref monitor) = evm_monitor {
        let retry_service = WatchRetryService::new(
            Arc::clone(&data_service),
            Arc::clone(monitor),
            WatchRetryConfig::default(),
        );
        tokio::spawn(retry_service.run());
    }

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
