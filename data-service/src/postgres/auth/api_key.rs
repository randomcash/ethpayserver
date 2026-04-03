//! PostgreSQL implementation for API key persistence.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use auth::{ApiKey, ApiKeyId, ApiKeyRepository, AuthError, Result, UserId};

/// PostgreSQL-backed API key repository.
pub struct PostgresApiKeyRepository {
    pool: PgPool,
}

impl PostgresApiKeyRepository {
    /// Create a new PostgreSQL API key repository.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl ApiKeyRepository for PostgresApiKeyRepository {
    async fn create_api_key(&self, key: &ApiKey) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO api_keys (id, user_id, name, key_hash, key_prefix, is_active, created_at, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
        )
        .bind(key.id.0)
        .bind(key.user_id.0)
        .bind(&key.name)
        .bind(&key.key_hash)
        .bind(&key.key_prefix)
        .bind(key.is_active)
        .bind(key.created_at)
        .bind(key.expires_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::Repository(e.to_string()))?;

        Ok(())
    }

    async fn get_api_key(&self, id: ApiKeyId) -> Result<Option<ApiKey>> {
        let row = sqlx::query_as::<_, ApiKeyRow>(
            "SELECT id, user_id, name, key_hash, key_prefix, is_active, created_at, last_used_at, expires_at FROM api_keys WHERE id = $1",
        )
        .bind(id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::Repository(e.to_string()))?;

        Ok(row.map(|r| r.into_api_key()))
    }

    async fn get_api_key_by_hash(&self, key_hash: &str) -> Result<Option<ApiKey>> {
        let row = sqlx::query_as::<_, ApiKeyRow>(
            "SELECT id, user_id, name, key_hash, key_prefix, is_active, created_at, last_used_at, expires_at FROM api_keys WHERE key_hash = $1",
        )
        .bind(key_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::Repository(e.to_string()))?;

        Ok(row.map(|r| r.into_api_key()))
    }

    async fn list_user_api_keys(&self, user_id: UserId) -> Result<Vec<ApiKey>> {
        let rows = sqlx::query_as::<_, ApiKeyRow>(
            "SELECT id, user_id, name, key_hash, key_prefix, is_active, created_at, last_used_at, expires_at FROM api_keys WHERE user_id = $1 ORDER BY created_at DESC",
        )
        .bind(user_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AuthError::Repository(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.into_api_key()).collect())
    }

    async fn revoke_api_key(&self, id: ApiKeyId) -> Result<()> {
        let result = sqlx::query("UPDATE api_keys SET is_active = false WHERE id = $1")
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(|e| AuthError::Repository(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(AuthError::ApiKeyNotFound("Key not found".to_string()));
        }

        Ok(())
    }

    async fn update_last_used(&self, id: ApiKeyId) -> Result<()> {
        let result = sqlx::query("UPDATE api_keys SET last_used_at = $1 WHERE id = $2")
            .bind(Utc::now())
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(|e| AuthError::Repository(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(AuthError::ApiKeyNotFound("Key not found".to_string()));
        }

        Ok(())
    }

    async fn delete_api_key(&self, id: ApiKeyId) -> Result<()> {
        let result = sqlx::query("DELETE FROM api_keys WHERE id = $1")
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(|e| AuthError::Repository(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(AuthError::ApiKeyNotFound("Key not found".to_string()));
        }

        Ok(())
    }

    async fn delete_api_keys_for_user(&self, user_id: UserId) -> Result<()> {
        sqlx::query("DELETE FROM api_keys WHERE user_id = $1")
            .bind(user_id.0)
            .execute(&self.pool)
            .await
            .map_err(|e| AuthError::Repository(e.to_string()))?;

        Ok(())
    }
}

/// Row structure for API key database queries.
#[derive(Debug, sqlx::FromRow)]
struct ApiKeyRow {
    id: Uuid,
    user_id: Uuid,
    name: String,
    key_hash: String,
    key_prefix: String,
    is_active: bool,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
}

impl ApiKeyRow {
    /// Convert row to ApiKey domain object.
    fn into_api_key(self) -> ApiKey {
        ApiKey {
            id: ApiKeyId(self.id),
            user_id: UserId(self.user_id),
            name: self.name,
            key_hash: self.key_hash,
            key_prefix: self.key_prefix,
            is_active: self.is_active,
            created_at: self.created_at,
            last_used_at: self.last_used_at,
            expires_at: self.expires_at,
        }
    }
}
