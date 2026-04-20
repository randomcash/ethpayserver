//! PostgreSQL implementation of the repository traits.

use chrono::{DateTime, Utc};
use sqlx::postgres::PgPool;
use uuid::Uuid;

mod auth;
mod conversions;
mod invoice;
mod payment;
mod payment_option;
mod store_payment_method;
mod store_wallet;
mod store_webhook;
mod token;
mod watched_address;
mod webhook_delivery;

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

    // ── API key deprecation (RCS-102) ────────────────────────────────────

    /// Look up an API key by its hash, returning auth-relevant fields.
    pub async fn get_api_key_auth_info(
        &self,
        key_hash: &str,
    ) -> Result<Option<ApiKeyAuthInfo>, sqlx::Error> {
        sqlx::query_as::<_, ApiKeyAuthInfo>(
            "SELECT id, user_id, is_active, deprecated_at, expires_at \
             FROM api_keys WHERE key_hash = $1",
        )
        .bind(key_hash)
        .fetch_optional(&self.pool)
        .await
    }

    /// List all API keys for a user, including deprecation info.
    pub async fn list_user_api_keys_full(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<ApiKeyFullInfo>, sqlx::Error> {
        sqlx::query_as::<_, ApiKeyFullInfo>(
            "SELECT id, name, key_prefix, is_active, created_at, \
                    last_used_at, expires_at, deprecated_at \
             FROM api_keys WHERE user_id = $1 ORDER BY created_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
    }

    /// Mark an API key as deprecated.
    pub async fn set_api_key_deprecated(
        &self,
        id: Uuid,
        deprecated_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE api_keys SET deprecated_at = $1 WHERE id = $2")
            .bind(deprecated_at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

/// Auth-relevant fields for API key validation.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ApiKeyAuthInfo {
    pub id: Uuid,
    pub user_id: Uuid,
    pub is_active: bool,
    pub deprecated_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Full API key info including deprecation status.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ApiKeyFullInfo {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub deprecated_at: Option<DateTime<Utc>>,
}
