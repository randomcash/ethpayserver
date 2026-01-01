//! Payment repository implementation.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::{PaymentReader, PaymentWriter, RepositoryError, RepositoryResult, sqlx_to_repo_error};
use types::{AssetType, InvoiceId, PaymentData};

use super::PgDataService;

/// Convert AssetType to database string.
fn asset_type_to_db(asset_type: AssetType) -> &'static str {
    match asset_type {
        AssetType::Native => "native",
        AssetType::ERC20 => "erc20",
    }
}

/// Convert database string to AssetType.
fn db_to_asset_type(s: &str) -> AssetType {
    match s {
        "erc20" => AssetType::ERC20,
        _ => AssetType::Native,
    }
}

/// Common SELECT columns for payment queries
const PAYMENT_SELECT_COLS: &str = r#"
    id, invoice_id, payment_option_id, chain_id, asset_type::text,
    amount::text, asset_symbol, token_address, tx_hash, block_number,
    detected_at, confirmed_at, from_address, reorged, extra,
    credited_amount::text, rate_used::text, rate_applied_at
"#;

#[async_trait]
impl PaymentReader for PgDataService {
    async fn get(&self, id: Uuid) -> RepositoryResult<Option<PaymentData>> {
        let query = format!(
            "SELECT {} FROM payments WHERE id = $1",
            PAYMENT_SELECT_COLS
        );
        let row = sqlx::query(&query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        match row {
            Some(r) => Ok(Some(try_row_to_payment(&r)?)),
            None => Ok(None),
        }
    }

    async fn get_for_invoice(&self, invoice_id: &InvoiceId) -> RepositoryResult<Vec<PaymentData>> {
        let query = format!(
            "SELECT {} FROM payments WHERE invoice_id = $1 ORDER BY detected_at DESC",
            PAYMENT_SELECT_COLS
        );
        let rows = sqlx::query(&query)
            .bind(invoice_id.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        rows.iter().map(try_row_to_payment).collect()
    }

    async fn get_awaiting_confirmation(&self) -> RepositoryResult<Vec<PaymentData>> {
        let query = format!(
            "SELECT {} FROM payments WHERE confirmed_at IS NULL AND reorged = FALSE ORDER BY detected_at ASC",
            PAYMENT_SELECT_COLS
        );
        let rows = sqlx::query(&query)
            .fetch_all(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        rows.iter().map(try_row_to_payment).collect()
    }

    async fn get_valid_for_invoice(&self, invoice_id: &InvoiceId) -> RepositoryResult<Vec<PaymentData>> {
        let query = format!(
            "SELECT {} FROM payments WHERE invoice_id = $1 AND reorged = FALSE ORDER BY detected_at DESC",
            PAYMENT_SELECT_COLS
        );
        let rows = sqlx::query(&query)
            .bind(invoice_id.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        rows.iter().map(try_row_to_payment).collect()
    }

    async fn has_valid_payments(&self, invoice_id: &InvoiceId) -> RepositoryResult<bool> {
        let row = sqlx::query(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM payments
                WHERE invoice_id = $1 AND reorged = FALSE
            ) as has_payments
            "#,
        )
        .bind(invoice_id.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row.get("has_payments"))
    }
}

#[async_trait]
impl PaymentWriter for PgDataService {
    async fn upsert(&self, payment: &PaymentData) -> RepositoryResult<()> {
        // Use ON CONFLICT (tx_hash, chain_id) to handle duplicate PaymentDetected events
        // (e.g., after service restart). This is the unique constraint in the DB.
        sqlx::query(
            r#"
            INSERT INTO payments (
                id, invoice_id, payment_option_id, chain_id, asset_type, amount, asset_symbol,
                token_address, tx_hash, block_number, detected_at, confirmed_at, from_address, extra,
                credited_amount, rate_used, rate_applied_at
            ) VALUES (
                $1, $2, $3, $4, $5::asset_type, $6::numeric, $7,
                $8, $9, $10, $11, $12, $13, $14,
                $15::numeric, $16::numeric, $17
            )
            ON CONFLICT (tx_hash, chain_id) DO UPDATE SET
                block_number = COALESCE(EXCLUDED.block_number, payments.block_number),
                confirmed_at = COALESCE(EXCLUDED.confirmed_at, payments.confirmed_at),
                extra = COALESCE(EXCLUDED.extra, payments.extra),
                credited_amount = COALESCE(EXCLUDED.credited_amount, payments.credited_amount),
                rate_used = COALESCE(EXCLUDED.rate_used, payments.rate_used),
                rate_applied_at = COALESCE(EXCLUDED.rate_applied_at, payments.rate_applied_at)
            "#,
        )
        .bind(payment.id)
        .bind(payment.invoice_id.as_str())
        .bind(payment.payment_option_id)
        .bind(payment.chain_id as i64)
        .bind(asset_type_to_db(payment.asset_type))
        .bind(&payment.amount)
        .bind(&payment.asset_symbol)
        .bind(&payment.token_address)
        .bind(&payment.tx_hash)
        .bind(payment.block_number.map(|n| n as i64))
        .bind(payment.detected_at)
        .bind(payment.confirmed_at)
        .bind(&payment.from_address)
        .bind(&payment.extra)
        .bind(&payment.credited_amount)
        .bind(&payment.rate_used)
        .bind(payment.rate_applied_at)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(())
    }

    async fn mark_confirmed(&self, id: Uuid, confirmed_at: DateTime<Utc>) -> RepositoryResult<()> {
        let result = sqlx::query(
            r#"
            UPDATE payments
            SET confirmed_at = $1
            WHERE id = $2 AND confirmed_at IS NULL
            "#,
        )
        .bind(confirmed_at)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound(format!(
                "Payment not found or already confirmed: {}",
                id
            )));
        }

        Ok(())
    }

    async fn mark_reorged(
        &self,
        invoice_id: &InvoiceId,
        chain_id: u64,
        fork_block: u64,
    ) -> RepositoryResult<u64> {
        let result = sqlx::query(
            r#"
            UPDATE payments
            SET reorged = TRUE, confirmed_at = NULL
            WHERE invoice_id = $1
              AND chain_id = $2
              AND block_number >= $3
              AND reorged = FALSE
            "#,
        )
        .bind(invoice_id.as_str())
        .bind(chain_id as i64)
        .bind(fork_block as i64)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(result.rows_affected())
    }
}

/// Convert a database row to PaymentData.
fn try_row_to_payment(row: &sqlx::postgres::PgRow) -> RepositoryResult<PaymentData> {
    let chain_id: i64 = row.get("chain_id");
    let block_number: Option<i64> = row.get("block_number");
    let asset_type_str: String = row.get("asset_type");

    Ok(PaymentData {
        id: row.get("id"),
        invoice_id: InvoiceId::from_string(row.get("invoice_id")),
        payment_option_id: row.get("payment_option_id"),
        chain_id: chain_id as u64,
        asset_type: db_to_asset_type(&asset_type_str),
        amount: row.get("amount"),
        asset_symbol: row.get("asset_symbol"),
        token_address: row.get("token_address"),
        tx_hash: row.get("tx_hash"),
        block_number: block_number.map(|n| n as u64),
        detected_at: row.get("detected_at"),
        confirmed_at: row.get("confirmed_at"),
        from_address: row.get("from_address"),
        reorged: row.get("reorged"),
        extra: row.get("extra"),
        credited_amount: row.get("credited_amount"),
        rate_used: row.get("rate_used"),
        rate_applied_at: row.get("rate_applied_at"),
    })
}

// =============================================================================
// Payment Events
// =============================================================================

use crate::PaymentEventWriter;

#[async_trait]
impl PaymentEventWriter for PgDataService {
    async fn create_event(
        &self,
        invoice_id: &InvoiceId,
        payment_id: Option<Uuid>,
        event_type: &str,
        event_data: Option<serde_json::Value>,
    ) -> RepositoryResult<Uuid> {
        let row = sqlx::query(
            r#"
            INSERT INTO payment_events (invoice_id, payment_id, event_type, event_data)
            VALUES ($1, $2, $3, $4)
            RETURNING id
            "#,
        )
        .bind(invoice_id.as_str())
        .bind(payment_id)
        .bind(event_type)
        .bind(&event_data)
        .fetch_one(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row.get("id"))
    }
}
