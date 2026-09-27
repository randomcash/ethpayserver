//! Webhook outbox repository implementation.

use async_trait::async_trait;
use sqlx::Row;
use uuid::Uuid;

use super::PgDataService;
use crate::{
    RepositoryResult, WebhookObligation, WebhookOutboxReader, WebhookOutboxWriter,
    sqlx_to_repo_error,
};

#[async_trait]
impl WebhookOutboxReader for PgDataService {
    async fn get_undispatched_obligations(
        &self,
        limit: i64,
    ) -> RepositoryResult<Vec<WebhookObligation>> {
        let rows = sqlx::query(
            r#"
            SELECT id, payment_id, invoice_id, event_type, created_at
            FROM webhook_outbox
            WHERE dispatched_at IS NULL
            ORDER BY created_at ASC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        rows.iter()
            .map(|row| {
                Ok(WebhookObligation {
                    id: row.try_get("id").map_err(sqlx_to_repo_error)?,
                    payment_id: row.try_get("payment_id").map_err(sqlx_to_repo_error)?,
                    invoice_id: row.try_get("invoice_id").map_err(sqlx_to_repo_error)?,
                    event_type: row.try_get("event_type").map_err(sqlx_to_repo_error)?,
                    created_at: row.try_get("created_at").map_err(sqlx_to_repo_error)?,
                })
            })
            .collect()
    }
}

#[async_trait]
impl WebhookOutboxWriter for PgDataService {
    async fn mark_obligation_dispatched(&self, id: Uuid) -> RepositoryResult<()> {
        sqlx::query("UPDATE webhook_outbox SET dispatched_at = now() WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        Ok(())
    }
}
