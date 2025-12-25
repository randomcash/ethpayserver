//! Store webhook repository implementation.

use async_trait::async_trait;
use sqlx::Row;
use uuid::Uuid;

use crate::{RepositoryResult, StoreWebhook, StoreWebhookReader, StoreWebhookWriter, sqlx_to_repo_error};
use super::PgDataService;

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

#[async_trait]
impl StoreWebhookReader for PgDataService {
    async fn get_enabled_webhook(&self, store_id: Uuid) -> RepositoryResult<Option<StoreWebhook>> {
        let row = sqlx::query(
            "SELECT * FROM store_webhooks WHERE store_id = $1 AND enabled = true",
        )
        .bind(store_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row.as_ref().map(row_to_webhook))
    }
}

#[async_trait]
impl StoreWebhookWriter for PgDataService {
    async fn upsert_webhook(
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

    async fn delete_webhook(&self, store_id: Uuid) -> RepositoryResult<bool> {
        let result = sqlx::query("DELETE FROM store_webhooks WHERE store_id = $1")
            .bind(store_id)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        Ok(result.rows_affected() > 0)
    }
}
