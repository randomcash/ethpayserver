//! PostgreSQL implementation of the repository traits.

use sqlx::postgres::PgPool;

mod auth;
mod conversions;
mod invoice;
mod payment;
mod payment_option;
mod payout;
mod refund;
mod store_payment_method;
mod store_wallet;
mod store_webhook;
mod token;
mod watched_address;

pub use auth::{ApiKeyRateLimitInfo, PostgresApiKeyRepository};
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

    /// Check database connectivity.
    pub async fn health_check(&self) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    /// Count total registered users.
    pub async fn count_users(&self) -> Result<i64, sqlx::Error> {
        let result: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await?;
        Ok(result.0)
    }

    /// Count total stores (non-archived).
    pub async fn count_stores(&self) -> Result<i64, sqlx::Error> {
        let result: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM stores WHERE archived = false")
            .fetch_one(&self.pool)
            .await?;
        Ok(result.0)
    }

    /// Look up an active API key's ID and rate limit by its hash.
    pub async fn get_api_key_rate_limit_by_hash(
        &self,
        key_hash: &str,
    ) -> ::auth::error::Result<Option<ApiKeyRateLimitInfo>> {
        PostgresApiKeyRepository::new(self.pool.clone())
            .get_rate_limit_by_hash(key_hash)
            .await
    }

    /// Update the per-key rate limit for an API key.
    pub async fn update_api_key_rate_limit(
        &self,
        id: ::auth::ApiKeyId,
        rpm: Option<i32>,
    ) -> ::auth::error::Result<()> {
        PostgresApiKeyRepository::new(self.pool.clone())
            .update_rate_limit(id, rpm)
            .await
    }

    /// List all API keys for a user, including rate_limit_rpm.
    pub async fn list_user_api_keys_with_rate_limit(
        &self,
        user_id: ::auth::UserId,
    ) -> ::auth::error::Result<Vec<(::auth::ApiKey, Option<i32>)>> {
        PostgresApiKeyRepository::new(self.pool.clone())
            .list_user_api_keys_with_rate_limit(user_id)
            .await
    }
}
