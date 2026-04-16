//! Webhook delivery repository implementation.

use async_trait::async_trait;
use sqlx::Row;
use uuid::Uuid;

use super::PgDataService;
use crate::{
    CreateDeliveryParams, RepositoryResult, WebhookDelivery, WebhookDeliveryReader,
    WebhookDeliveryWriter, sqlx_to_repo_error,
};

fn row_to_delivery(row: &sqlx::postgres::PgRow) -> WebhookDelivery {
    WebhookDelivery {
        id: row.get("id"),
        store_id: row.get("store_id"),
        event_type: row.get("event_type"),
        payload: row.get("payload"),
        http_status: row.get("http_status"),
        response_body: row.get("response_body"),
        latency_ms: row.get("latency_ms"),
        success: row.get("success"),
        error_message: row.get("error_message"),
        attempt_number: row.get("attempt_number"),
        created_at: row.get("created_at"),
    }
}

#[async_trait]
impl WebhookDeliveryReader for PgDataService {
    async fn list_deliveries(
        &self,
        store_id: Uuid,
        event_type: Option<&str>,
        success: Option<bool>,
        limit: i64,
        offset: i64,
    ) -> RepositoryResult<(i64, Vec<WebhookDelivery>)> {
        // Build dynamic query for count
        let mut count_sql =
            String::from("SELECT COUNT(*) FROM webhook_deliveries WHERE store_id = $1");
        let mut query_sql = String::from("SELECT * FROM webhook_deliveries WHERE store_id = $1");

        let mut param_idx = 2;
        if event_type.is_some() {
            count_sql.push_str(&format!(" AND event_type = ${param_idx}"));
            query_sql.push_str(&format!(" AND event_type = ${param_idx}"));
            param_idx += 1;
        }
        if success.is_some() {
            count_sql.push_str(&format!(" AND success = ${param_idx}"));
            query_sql.push_str(&format!(" AND success = ${param_idx}"));
            param_idx += 1;
        }

        query_sql.push_str(&format!(
            " ORDER BY created_at DESC LIMIT ${param_idx} OFFSET ${}",
            param_idx + 1
        ));

        // Execute count query
        let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql).bind(store_id);
        if let Some(et) = event_type {
            count_query = count_query.bind(et);
        }
        if let Some(s) = success {
            count_query = count_query.bind(s);
        }
        let total = count_query
            .fetch_one(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        // Execute main query
        let mut main_query = sqlx::query(&query_sql).bind(store_id);
        if let Some(et) = event_type {
            main_query = main_query.bind(et);
        }
        if let Some(s) = success {
            main_query = main_query.bind(s);
        }
        main_query = main_query.bind(limit).bind(offset);

        let rows = main_query
            .fetch_all(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        let deliveries = rows.iter().map(row_to_delivery).collect();
        Ok((total, deliveries))
    }

    async fn get_delivery(&self, delivery_id: Uuid) -> RepositoryResult<Option<WebhookDelivery>> {
        let row = sqlx::query("SELECT * FROM webhook_deliveries WHERE id = $1")
            .bind(delivery_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        Ok(row.as_ref().map(row_to_delivery))
    }
}

#[async_trait]
impl WebhookDeliveryWriter for PgDataService {
    async fn create_delivery(
        &self,
        params: CreateDeliveryParams,
    ) -> RepositoryResult<WebhookDelivery> {
        let row = sqlx::query(
            r#"
            INSERT INTO webhook_deliveries
                (store_id, event_type, payload, http_status, response_body,
                 latency_ms, success, error_message, attempt_number)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING *
            "#,
        )
        .bind(params.store_id)
        .bind(&params.event_type)
        .bind(&params.payload)
        .bind(params.http_status)
        .bind(&params.response_body)
        .bind(params.latency_ms)
        .bind(params.success)
        .bind(&params.error_message)
        .bind(params.attempt_number)
        .fetch_one(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row_to_delivery(&row))
    }
}
