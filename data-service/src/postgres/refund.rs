//! Refund repository implementation.

use async_trait::async_trait;
use sqlx::Row;
use uuid::Uuid;

use crate::{RefundReader, RefundWriter, RepositoryResult, sqlx_to_repo_error};
use types::{InvoiceId, RefundData, RefundStatus, StoreId};

use super::PgDataService;

fn db_to_refund_status(s: &str) -> RefundStatus {
    s.parse().unwrap_or(RefundStatus::Failed)
}

fn try_row_to_refund(row: &sqlx::postgres::PgRow) -> RepositoryResult<RefundData> {
    Ok(RefundData {
        id: row.try_get("id").map_err(sqlx_to_repo_error)?,
        invoice_id: InvoiceId::from_string(
            row.try_get::<String, _>("invoice_id")
                .map_err(sqlx_to_repo_error)?,
        ),
        payment_id: row.try_get("payment_id").map_err(sqlx_to_repo_error)?,
        store_id: StoreId(row.try_get("store_id").map_err(sqlx_to_repo_error)?),
        to_address: row.try_get("to_address").map_err(sqlx_to_repo_error)?,
        chain_id: row
            .try_get::<i64, _>("chain_id")
            .map_err(sqlx_to_repo_error)? as u64,
        asset_type: row.try_get("asset_type").map_err(sqlx_to_repo_error)?,
        asset_symbol: row.try_get("asset_symbol").map_err(sqlx_to_repo_error)?,
        token_address: row.try_get("token_address").map_err(sqlx_to_repo_error)?,
        amount: row.try_get("amount").map_err(sqlx_to_repo_error)?,
        tx_hash: row.try_get("tx_hash").map_err(sqlx_to_repo_error)?,
        status: db_to_refund_status(
            &row.try_get::<String, _>("status")
                .map_err(sqlx_to_repo_error)?,
        ),
        fee_amount: row.try_get("fee_amount").map_err(sqlx_to_repo_error)?,
        reason: row.try_get("reason").map_err(sqlx_to_repo_error)?,
        error_message: row.try_get("error_message").map_err(sqlx_to_repo_error)?,
        created_at: row.try_get("created_at").map_err(sqlx_to_repo_error)?,
        confirmed_at: row.try_get("confirmed_at").map_err(sqlx_to_repo_error)?,
    })
}

#[async_trait]
impl RefundReader for PgDataService {
    async fn get_refund(&self, id: Uuid) -> RepositoryResult<Option<RefundData>> {
        let row = sqlx::query("SELECT * FROM refunds WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        match row {
            Some(r) => Ok(Some(try_row_to_refund(&r)?)),
            None => Ok(None),
        }
    }

    async fn get_refunds_for_invoice(
        &self,
        invoice_id: &InvoiceId,
    ) -> RepositoryResult<Vec<RefundData>> {
        let rows =
            sqlx::query("SELECT * FROM refunds WHERE invoice_id = $1 ORDER BY created_at DESC")
                .bind(invoice_id.as_str())
                .fetch_all(&self.pool)
                .await
                .map_err(sqlx_to_repo_error)?;

        rows.iter().map(try_row_to_refund).collect()
    }

    async fn get_refunds_for_store(
        &self,
        store_id: StoreId,
        limit: i64,
        offset: i64,
    ) -> RepositoryResult<(i64, Vec<RefundData>)> {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM refunds WHERE store_id = $1")
            .bind(store_id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        let rows = sqlx::query(
            "SELECT * FROM refunds WHERE store_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3",
        )
        .bind(store_id.0)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        let refunds: Vec<RefundData> = rows
            .iter()
            .map(try_row_to_refund)
            .collect::<Result<_, _>>()?;
        Ok((count.0, refunds))
    }

    async fn get_active_refunds(&self) -> RepositoryResult<Vec<RefundData>> {
        let rows = sqlx::query(
            "SELECT * FROM refunds WHERE status IN ('pending', 'broadcasting') ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        rows.iter().map(try_row_to_refund).collect()
    }
}

#[async_trait]
impl RefundWriter for PgDataService {
    async fn create_refund(&self, refund: &RefundData) -> RepositoryResult<()> {
        sqlx::query(
            "INSERT INTO refunds (id, invoice_id, payment_id, store_id, to_address, chain_id, asset_type, asset_symbol, token_address, amount, tx_hash, status, fee_amount, reason, error_message, created_at, confirmed_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
        )
        .bind(refund.id)
        .bind(refund.invoice_id.as_str())
        .bind(refund.payment_id)
        .bind(refund.store_id.0)
        .bind(&refund.to_address)
        .bind(refund.chain_id as i64)
        .bind(&refund.asset_type)
        .bind(&refund.asset_symbol)
        .bind(&refund.token_address)
        .bind(&refund.amount)
        .bind(&refund.tx_hash)
        .bind(refund.status.as_str())
        .bind(&refund.fee_amount)
        .bind(&refund.reason)
        .bind(&refund.error_message)
        .bind(refund.created_at)
        .bind(refund.confirmed_at)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(())
    }

    async fn update_refund_status(
        &self,
        id: Uuid,
        status: RefundStatus,
        tx_hash: Option<&str>,
        fee_amount: Option<&str>,
        error_message: Option<&str>,
    ) -> RepositoryResult<()> {
        sqlx::query(
            "UPDATE refunds SET status = $2, tx_hash = COALESCE($3, tx_hash), fee_amount = COALESCE($4, fee_amount), error_message = COALESCE($5, error_message) WHERE id = $1",
        )
        .bind(id)
        .bind(status.as_str())
        .bind(tx_hash)
        .bind(fee_amount)
        .bind(error_message)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(())
    }

    async fn confirm_refund(&self, id: Uuid) -> RepositoryResult<()> {
        sqlx::query("UPDATE refunds SET status = 'confirmed', confirmed_at = NOW() WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        Ok(())
    }
}
