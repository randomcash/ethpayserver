//! Database migration binary.
//!
//! Run migrations with:
//! ```bash
//! DATABASE_URL="postgres://..." cargo run --bin migrate
//! ```

use anyhow::Result;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use data_service::PgDataService;

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file if present
    let _ = dotenvy::dotenv();

    // Initialize tracing
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Get database URL
    let database_url = std::env::var("DATABASE_URL")
        .map_err(|_| anyhow::anyhow!("DATABASE_URL environment variable is required"))?;

    // Connect to database
    tracing::info!("Connecting to database...");
    let data_service = PgDataService::connect(&database_url).await?;
    tracing::info!("Database connected");

    // Run migrations
    tracing::info!("Running database migrations...");
    sqlx::migrate!("../data-service/migrations/postgres")
        .run(data_service.pool())
        .await?;
    tracing::info!("Database migrations completed");

    Ok(())
}
