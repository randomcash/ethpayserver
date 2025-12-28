//! Invoice repository implementation.

use async_trait::async_trait;
use sqlx::Row;
use uuid::Uuid;

use futures::stream::BoxStream;
use futures::StreamExt;

use crate::{
    InvoiceQueryParams, InvoiceReader, InvoiceWriter, RepositoryError, RepositoryResult,
    sqlx_to_repo_error,
};
use types::{InvoiceData, InvoiceId, InvoiceStatus, StoreId};

use super::conversions::{status_to_db, try_db_to_status};
use super::PgDataService;

#[async_trait]
impl InvoiceReader for PgDataService {
    async fn get(&self, id: &InvoiceId) -> RepositoryResult<Option<InvoiceData>> {
        let row = sqlx::query(
            r#"
            SELECT
                id, store_id, currency, status::text, amount::text,
                amount_received::text, created_at, expires_at, metadata, extra
            FROM invoices
            WHERE id = $1
            "#,
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        match row {
            Some(r) => Ok(Some(try_row_to_invoice(&r)?)),
            None => Ok(None),
        }
    }

    async fn query(&self, params: &InvoiceQueryParams) -> RepositoryResult<(i64, Vec<InvoiceData>)> {
        // Build dynamic query for filtering
        let mut conditions = Vec::new();
        let mut bind_idx = 1;

        if params.store_id.is_some() {
            conditions.push(format!("store_id = ${}", bind_idx));
            bind_idx += 1;
        }
        if params.status.is_some() {
            conditions.push(format!("status = ${}::invoice_status", bind_idx));
            bind_idx += 1;
        }
        if params.currency.is_some() {
            conditions.push(format!("currency = ${}", bind_idx));
            bind_idx += 1;
        }
        if params.created_after.is_some() {
            conditions.push(format!("created_at >= ${}", bind_idx));
            bind_idx += 1;
        }
        if params.created_before.is_some() {
            conditions.push(format!("created_at <= ${}", bind_idx));
            bind_idx += 1;
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        // Count query
        let count_sql = format!("SELECT COUNT(*) as count FROM invoices {}", where_clause);

        // Data query
        let data_sql = format!(
            r#"
            SELECT
                id, store_id, currency, status::text, amount::text,
                amount_received::text, created_at, expires_at, metadata, extra
            FROM invoices
            {}
            ORDER BY created_at DESC
            LIMIT ${} OFFSET ${}
            "#,
            where_clause,
            bind_idx,
            bind_idx + 1
        );

        // Build and execute count query
        let mut count_query = sqlx::query(&count_sql);
        if let Some(store_id) = params.store_id {
            count_query = count_query.bind(store_id.0);
        }
        if let Some(status) = params.status {
            count_query = count_query.bind(status_to_db(status));
        }
        if let Some(ref currency) = params.currency {
            count_query = count_query.bind(currency);
        }
        if let Some(after) = params.created_after {
            count_query = count_query.bind(after);
        }
        if let Some(before) = params.created_before {
            count_query = count_query.bind(before);
        }

        let count_row = count_query
            .fetch_one(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;
        let total: i64 = count_row.get("count");

        // Build and execute data query
        let mut data_query = sqlx::query(&data_sql);
        if let Some(store_id) = params.store_id {
            data_query = data_query.bind(store_id.0);
        }
        if let Some(status) = params.status {
            data_query = data_query.bind(status_to_db(status));
        }
        if let Some(ref currency) = params.currency {
            data_query = data_query.bind(currency);
        }
        if let Some(after) = params.created_after {
            data_query = data_query.bind(after);
        }
        if let Some(before) = params.created_before {
            data_query = data_query.bind(before);
        }
        data_query = data_query.bind(params.limit).bind(params.offset);

        let rows = data_query
            .fetch_all(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        let invoices: Result<Vec<InvoiceData>, _> =
            rows.iter().map(try_row_to_invoice).collect();

        Ok((total, invoices?))
    }

    async fn get_expired(&self) -> RepositoryResult<Vec<InvoiceData>> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, store_id, currency, status::text, amount::text,
                amount_received::text, created_at, expires_at, metadata, extra
            FROM invoices
            WHERE status IN ('pending', 'processing', 'partially_paid')
              AND expires_at < NOW()
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        let invoices: Result<Vec<InvoiceData>, _> =
            rows.iter().map(try_row_to_invoice).collect();

        invoices
    }

    fn stream_expired_pending(&self) -> BoxStream<'_, RepositoryResult<InvoiceId>> {
        Box::pin(
            sqlx::query_scalar::<_, String>(
                r#"
                SELECT id
                FROM invoices
                WHERE status = 'pending'
                  AND expires_at < NOW()
                "#,
            )
            .fetch(&self.pool)
            .map(|result| {
                result
                    .map(InvoiceId::from_string)
                    .map_err(sqlx_to_repo_error)
            }),
        )
    }
}

#[async_trait]
impl InvoiceWriter for PgDataService {
    async fn upsert(&self, invoice: &InvoiceData) -> RepositoryResult<()> {
        sqlx::query(
            r#"
            INSERT INTO invoices (
                id, store_id, currency, status, amount, amount_received,
                created_at, expires_at, metadata, extra
            ) VALUES (
                $1, $2, $3, $4::invoice_status, $5::numeric, $6::numeric,
                $7, $8, $9, $10
            )
            ON CONFLICT (id) DO UPDATE SET
                status = EXCLUDED.status,
                amount = EXCLUDED.amount,
                amount_received = EXCLUDED.amount_received,
                expires_at = EXCLUDED.expires_at,
                metadata = EXCLUDED.metadata,
                extra = EXCLUDED.extra
            "#,
        )
        .bind(invoice.id.as_str())
        .bind(invoice.store_id.0)
        .bind(&invoice.currency)
        .bind(status_to_db(invoice.status))
        .bind(&invoice.amount)
        .bind(&invoice.amount_received)
        .bind(invoice.created_at)
        .bind(invoice.expires_at)
        .bind(&invoice.metadata)
        .bind(&invoice.extra)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(())
    }

    async fn update_status(&self, id: &InvoiceId, status: InvoiceStatus) -> RepositoryResult<()> {
        let result = sqlx::query("UPDATE invoices SET status = $1::invoice_status WHERE id = $2")
            .bind(status_to_db(status))
            .bind(id.as_str())
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound(format!(
                "Invoice not found: {}",
                id.as_str()
            )));
        }

        Ok(())
    }

    async fn update_amount_received(&self, id: &InvoiceId, amount: &str) -> RepositoryResult<()> {
        let result = sqlx::query("UPDATE invoices SET amount_received = $1::numeric WHERE id = $2")
            .bind(amount)
            .bind(id.as_str())
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound(format!(
                "Invoice not found: {}",
                id.as_str()
            )));
        }

        Ok(())
    }

    async fn expire(&self, id: &InvoiceId) -> RepositoryResult<bool> {
        let result = sqlx::query(
            r#"
            UPDATE invoices
            SET status = 'expired'::invoice_status
            WHERE id = $1 AND status = 'pending'
            "#,
        )
        .bind(id.as_str())
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(result.rows_affected() > 0)
    }
}

/// Convert a database row to InvoiceData.
fn try_row_to_invoice(row: &sqlx::postgres::PgRow) -> RepositoryResult<InvoiceData> {
    let store_id: Uuid = row.get("store_id");
    Ok(InvoiceData {
        id: InvoiceId::from_string(row.get("id")),
        store_id: StoreId(store_id),
        currency: row.get("currency"),
        status: try_db_to_status(row.get("status"))?,
        amount: row.get("amount"),
        amount_received: row.get("amount_received"),
        created_at: row.get("created_at"),
        expires_at: row.get("expires_at"),
        metadata: row.get("metadata"),
        extra: row.get("extra"),
    })
}
