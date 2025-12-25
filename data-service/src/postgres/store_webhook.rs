//! Store webhook repository implementation.

use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::{RepositoryResult, sqlx_to_repo_error};
use super::PgDataService;

/// Store webhook configuration for invoice notifications.
#[derive(Debug, Clone)]
pub struct StoreWebhook {
    pub id: Uuid,
    pub store_id: Uuid,
    pub webhook_url: String,
    pub webhook_secret: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn row_to_webhook(row: &sqlx::postgres::PgRow) -> StoreWebhook {
    StoreWebhook {
        id: row.get("id"),
        store_id: row.get("store_id"),
        webhook_url: row.get("webhook_url"),
        webhook_secret: row.get("webhook_secret"),
        enabled: row.get("enabled"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

impl PgDataService {
    /// Get enabled webhook configuration for a store.
    /// Returns None if webhook is not configured or is disabled.
    pub async fn get_enabled_store_webhook(&self, store_id: Uuid) -> RepositoryResult<Option<StoreWebhook>> {
        let row = sqlx::query(
            "SELECT * FROM store_webhooks WHERE store_id = $1 AND enabled = true",
        )
        .bind(store_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row.as_ref().map(row_to_webhook))
    }

    /// Create or update webhook configuration for a store.
    pub async fn upsert_store_webhook(
        &self,
        store_id: Uuid,
        webhook_url: &str,
        webhook_secret: &str,
        enabled: bool,
    ) -> RepositoryResult<StoreWebhook> {
        let row = sqlx::query(
            r#"
            INSERT INTO store_webhooks (store_id, webhook_url, webhook_secret, enabled)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (store_id) DO UPDATE SET
                webhook_url = EXCLUDED.webhook_url,
                webhook_secret = EXCLUDED.webhook_secret,
                enabled = EXCLUDED.enabled,
                updated_at = NOW()
            RETURNING *
            "#,
        )
        .bind(store_id)
        .bind(webhook_url)
        .bind(webhook_secret)
        .bind(enabled)
        .fetch_one(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row_to_webhook(&row))
    }

    /// Delete webhook configuration for a store.
    pub async fn delete_store_webhook(&self, store_id: Uuid) -> RepositoryResult<bool> {
        let result = sqlx::query("DELETE FROM store_webhooks WHERE store_id = $1")
            .bind(store_id)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        Ok(result.rows_affected() > 0)
    }
}
