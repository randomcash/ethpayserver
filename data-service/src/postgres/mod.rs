//! PostgreSQL implementation of the repository traits.

use metrics::histogram;
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgPool, Postgres};

mod auth;
mod conversions;
mod invoice;
mod payment;
mod payment_option;
mod payout;
mod refund;
mod store_payment_method;
mod store_settings;
mod store_wallet;
mod store_webhook;
mod token;
mod watched_address;

pub use watched_address::PendingWatch;

#[cfg(test)]
mod integration_tests;
#[cfg(test)]
mod tests;

/// PostgreSQL data service implementation.
#[derive(Clone)]
pub struct PgDataService {
    pool: PgPool,
}

impl PgDataService {
    /// Create a new PgDataService with the given connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Connect to PostgreSQL with the given database URL.
    pub async fn connect(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPool::connect(database_url).await?;
        Ok(Self::new(pool))
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Acquire a connection from the pool, recording wait duration as a metric.
    pub async fn acquire(&self) -> Result<PoolConnection<Postgres>, sqlx::Error> {
        let start = std::time::Instant::now();
        let conn = self.pool.acquire().await;
        histogram!("ethpayserver_db_pool_wait_duration_seconds")
            .record(start.elapsed().as_secs_f64());
        conn
    }

    /// Check database connectivity.
    pub async fn health_check(&self) -> Result<(), sqlx::Error> {
        let mut conn = self.acquire().await?;
        sqlx::query("SELECT 1").execute(&mut *conn).await?;
        Ok(())
    }

    /// Count total registered users.
    pub async fn count_users(&self) -> Result<i64, sqlx::Error> {
        let mut conn = self.acquire().await?;
        let result: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&mut *conn)
            .await?;
        Ok(result.0)
    }

    /// Count total stores (non-archived).
    pub async fn count_stores(&self) -> Result<i64, sqlx::Error> {
        let mut conn = self.acquire().await?;
        let result: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM stores WHERE archived = false")
            .fetch_one(&mut *conn)
            .await?;
        Ok(result.0)
    }
}
