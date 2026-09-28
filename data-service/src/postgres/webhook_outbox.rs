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
    async fn claim_undispatched_obligations(
        &self,
        limit: i64,
        visibility_secs: i64,
    ) -> RepositoryResult<Vec<WebhookObligation>> {
        // `FOR UPDATE SKIP LOCKED` inside the CTE is what makes this safe
        // between two drain instances: each claiming query only ever picks
        // rows the other isn't already holding a row lock on, so the same
        // obligation cannot end up in two claimants' result sets even if
        // both queries run at the same instant. The outer `UPDATE` stamps
        // `claimed_until` on exactly the rows the CTE picked, so the claim
        // and the read that decided it happen in one statement.
        let rows = sqlx::query(
            r#"
            WITH claimable AS (
                SELECT id
                FROM webhook_outbox
                WHERE dispatched_at IS NULL
                  AND (claimed_until IS NULL OR claimed_until < now())
                ORDER BY created_at ASC
                LIMIT $1
                FOR UPDATE SKIP LOCKED
            )
            UPDATE webhook_outbox
            SET claimed_until = now() + make_interval(secs => $2::double precision)
            FROM claimable
            WHERE webhook_outbox.id = claimable.id
            RETURNING webhook_outbox.id, webhook_outbox.payment_id, webhook_outbox.invoice_id,
                      webhook_outbox.event_type, webhook_outbox.created_at
            "#,
        )
        .bind(limit)
        .bind(visibility_secs)
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
