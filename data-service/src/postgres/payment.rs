//! Payment repository implementation.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::{PaymentReader, PaymentWriter, RepositoryError, RepositoryResult, sqlx_to_repo_error};
use types::{InvoiceId, PaymentData};

use super::conversions::{try_db_to_network, try_network_to_db};
use super::PgDataService;

#[async_trait]
impl PaymentReader for PgDataService {
    async fn get(&self, id: Uuid) -> RepositoryResult<Option<PaymentData>> {
        let row = sqlx::query(
            r#"
            SELECT
                id, invoice_id, network::text, amount_value::text,
                asset_symbol, tx_hash, block_number, confirmations,
                detected_at, confirmed_at, from_address, extra
            FROM payments
            WHERE id = $1
            "#,
        )
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
        let rows = sqlx::query(
            r#"
            SELECT
                id, invoice_id, network::text, amount_value::text,
                asset_symbol, tx_hash, block_number, confirmations,
                detected_at, confirmed_at, from_address, extra
            FROM payments
            WHERE invoice_id = $1
            ORDER BY detected_at DESC
            "#,
        )
        .bind(invoice_id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        let payments: Result<Vec<PaymentData>, _> = rows.iter().map(try_row_to_payment).collect();

        payments
    }

    async fn get_unconfirmed(&self, min_confirmations: u32) -> RepositoryResult<Vec<PaymentData>> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, invoice_id, network::text, amount_value::text,
                asset_symbol, tx_hash, block_number, confirmations,
                detected_at, confirmed_at, from_address, extra
            FROM payments
            WHERE confirmations < $1
            ORDER BY detected_at ASC
            "#,
        )
        .bind(min_confirmations as i32)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        let payments: Result<Vec<PaymentData>, _> = rows.iter().map(try_row_to_payment).collect();

        payments
    }
}

#[async_trait]
impl PaymentWriter for PgDataService {
    async fn upsert(&self, payment: &PaymentData) -> RepositoryResult<()> {
        sqlx::query(
            r#"
            INSERT INTO payments (
                id, invoice_id, network, amount_value, asset_symbol,
                tx_hash, block_number, confirmations, detected_at,
                confirmed_at, from_address, extra, asset_type
            ) VALUES (
                $1, $2, $3::network, $4::numeric, $5,
                $6, $7, $8, $9, $10, $11, $12, 'native'::asset_type
            )
            ON CONFLICT (id) DO UPDATE SET
                amount_value = EXCLUDED.amount_value,
                block_number = EXCLUDED.block_number,
                confirmations = EXCLUDED.confirmations,
                confirmed_at = EXCLUDED.confirmed_at,
                extra = EXCLUDED.extra
            "#,
        )
        .bind(payment.id)
        .bind(payment.invoice_id.as_str())
        .bind(try_network_to_db(payment.network)?)
        .bind(&payment.amount)
        .bind(&payment.asset_symbol)
        .bind(&payment.tx_hash)
        .bind(payment.block_number.map(|n| n as i64))
        .bind(payment.confirmations as i32)
        .bind(payment.detected_at)
        .bind(payment.confirmed_at)
        .bind(&payment.from_address)
        .bind(&payment.extra)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(())
    }

    async fn update_confirmations(
        &self,
        id: Uuid,
        confirmations: u32,
        confirmed_at: Option<DateTime<Utc>>,
    ) -> RepositoryResult<()> {
        let result = sqlx::query(
            r#"
            UPDATE payments
            SET confirmations = $1, confirmed_at = COALESCE($2, confirmed_at)
            WHERE id = $3
            "#,
        )
        .bind(confirmations as i32)
        .bind(confirmed_at)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound(format!(
                "Payment not found: {}",
                id
            )));
        }

        Ok(())
    }
}

/// Convert a database row to PaymentData.
fn try_row_to_payment(row: &sqlx::postgres::PgRow) -> RepositoryResult<PaymentData> {
    let block_number: Option<i64> = row.get("block_number");
    let confirmations: i32 = row.get("confirmations");

    Ok(PaymentData {
        id: row.get("id"),
        invoice_id: InvoiceId::from_string(row.get("invoice_id")),
        network: try_db_to_network(row.get("network"))?,
        amount: row.get("amount_value"),
        asset_symbol: row.get("asset_symbol"),
        tx_hash: row.get("tx_hash"),
        block_number: block_number.map(|n| n as u64),
        confirmations: confirmations as u32,
        detected_at: row.get("detected_at"),
        confirmed_at: row.get("confirmed_at"),
        from_address: row.get("from_address"),
        extra: row.get("extra"),
    })
}
