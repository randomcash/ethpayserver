//! Webhook delivery repository implementation.

use async_trait::async_trait;
use sqlx::Row;
use uuid::Uuid;

use super::PgDataService;
use crate::{
    RepositoryResult, UpsertDeliveryParams, WebhookDeliveryData, WebhookDeliveryReader,
    WebhookDeliveryStatus, WebhookDeliveryWriter, sqlx_to_repo_error,
};

fn row_to_delivery(row: &sqlx::postgres::PgRow) -> RepositoryResult<WebhookDeliveryData> {
    let status: String = row.try_get("status").map_err(sqlx_to_repo_error)?;

    Ok(WebhookDeliveryData {
        id: row.try_get("id").map_err(sqlx_to_repo_error)?,
        store_webhook_id: row
            .try_get("store_webhook_id")
            .map_err(sqlx_to_repo_error)?,
        store_id: row.try_get("store_id").map_err(sqlx_to_repo_error)?,
        invoice_id: row.try_get("invoice_id").map_err(sqlx_to_repo_error)?,
        event_type: row.try_get("event_type").map_err(sqlx_to_repo_error)?,
        status: status.parse().unwrap_or(WebhookDeliveryStatus::Failed),
        attempts: row.try_get("attempts").map_err(sqlx_to_repo_error)?,
        max_attempts: row.try_get("max_attempts").map_err(sqlx_to_repo_error)?,
        last_error: row.try_get("last_error").map_err(sqlx_to_repo_error)?,
        payload: row.try_get("payload").map_err(sqlx_to_repo_error)?,
        created_at: row.try_get("created_at").map_err(sqlx_to_repo_error)?,
        updated_at: row.try_get("updated_at").map_err(sqlx_to_repo_error)?,
    })
}

#[async_trait]
impl WebhookDeliveryWriter for PgDataService {
    async fn upsert_delivery(&self, params: UpsertDeliveryParams) -> RepositoryResult<()> {
        // ON CONFLICT (id) is what makes retries of the same job update one
        // row instead of inserting a new one: store_webhook_id, invoice_id,
        // event_type and payload only take effect on the initial insert, the
        // update arm only ever moves status/attempts/last_error forward.
        sqlx::query(
            r#"
            INSERT INTO webhook_deliveries
                (id, store_webhook_id, invoice_id, event_type, status, attempts, max_attempts, last_error, payload)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (id) DO UPDATE SET
                status = EXCLUDED.status,
                attempts = EXCLUDED.attempts,
                last_error = EXCLUDED.last_error
            "#,
        )
        .bind(params.id)
        .bind(params.store_webhook_id)
        .bind(&params.invoice_id)
        .bind(&params.event_type)
        .bind(params.status.as_str())
        .bind(params.attempts)
        .bind(params.max_attempts)
        .bind(&params.last_error)
        .bind(&params.payload)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(())
    }
}

#[async_trait]
impl WebhookDeliveryReader for PgDataService {
    async fn get_delivery(&self, id: Uuid) -> RepositoryResult<Option<WebhookDeliveryData>> {
        let row = sqlx::query(
            "SELECT wd.*, sw.store_id AS store_id \
             FROM webhook_deliveries wd \
             JOIN store_webhooks sw ON sw.id = wd.store_webhook_id \
             WHERE wd.id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        row.as_ref().map(row_to_delivery).transpose()
    }

    async fn list_deliveries_for_invoice(
        &self,
        invoice_id: &str,
        limit: i64,
        offset: i64,
    ) -> RepositoryResult<(i64, Vec<WebhookDeliveryData>)> {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM webhook_deliveries wd \
             JOIN store_webhooks sw ON sw.id = wd.store_webhook_id \
             WHERE wd.invoice_id = $1",
        )
        .bind(invoice_id)
        .fetch_one(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        let rows = sqlx::query(
            "SELECT wd.*, sw.store_id AS store_id \
             FROM webhook_deliveries wd \
             JOIN store_webhooks sw ON sw.id = wd.store_webhook_id \
             WHERE wd.invoice_id = $1 \
             ORDER BY wd.created_at DESC LIMIT $2 OFFSET $3",
        )
        .bind(invoice_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        let deliveries = rows.iter().map(row_to_delivery).collect::<Result<_, _>>()?;
        Ok((count.0, deliveries))
    }

    async fn list_deliveries_for_store(
        &self,
        store_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> RepositoryResult<(i64, Vec<WebhookDeliveryData>)> {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM webhook_deliveries wd \
             JOIN store_webhooks sw ON sw.id = wd.store_webhook_id \
             WHERE sw.store_id = $1",
        )
        .bind(store_id)
        .fetch_one(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        let rows = sqlx::query(
            "SELECT wd.*, sw.store_id AS store_id \
             FROM webhook_deliveries wd \
             JOIN store_webhooks sw ON sw.id = wd.store_webhook_id \
             WHERE sw.store_id = $1 \
             ORDER BY wd.created_at DESC LIMIT $2 OFFSET $3",
        )
        .bind(store_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        let deliveries = rows.iter().map(row_to_delivery).collect::<Result<_, _>>()?;
        Ok((count.0, deliveries))
    }
}
