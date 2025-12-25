//! Payment repository implementation.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::{PaymentReader, PaymentWriter, RepositoryError, RepositoryResult, sqlx_to_repo_error};
use types::{AssetType, InvoiceId, PaymentData};

use super::PgDataService;
use super::conversions::{try_db_to_network, try_network_to_db};

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

#[async_trait]
impl PaymentReader for PgDataService {
    async fn get(&self, id: Uuid) -> RepositoryResult<Option<PaymentData>> {
        let row = sqlx::query(
            r#"
            SELECT
                id, invoice_id, network::text, asset_type::text,
                amount_value::text, asset_symbol, tx_hash, block_number,
                detected_at, confirmed_at, from_address, reorged, extra
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
                id, invoice_id, network::text, asset_type::text,
                amount_value::text, asset_symbol, tx_hash, block_number,
                detected_at, confirmed_at, from_address, reorged, extra
            FROM payments
            WHERE invoice_id = $1
            ORDER BY detected_at DESC
            "#,
        )
        .bind(invoice_id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        rows.iter().map(try_row_to_payment).collect()
    }

    async fn get_awaiting_confirmation(&self) -> RepositoryResult<Vec<PaymentData>> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, invoice_id, network::text, asset_type::text,
                amount_value::text, asset_symbol, tx_hash, block_number,
                detected_at, confirmed_at, from_address, reorged, extra
            FROM payments
            WHERE confirmed_at IS NULL AND reorged = FALSE
            ORDER BY detected_at ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        rows.iter().map(try_row_to_payment).collect()
    }

    async fn get_valid_for_invoice(&self, invoice_id: &InvoiceId) -> RepositoryResult<Vec<PaymentData>> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, invoice_id, network::text, asset_type::text,
                amount_value::text, asset_symbol, tx_hash, block_number,
                detected_at, confirmed_at, from_address, reorged, extra
            FROM payments
            WHERE invoice_id = $1 AND reorged = FALSE
            ORDER BY detected_at DESC
            "#,
        )
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
        // Store token_address in extra if present
        let extra = match (&payment.extra, &payment.token_address) {
            (Some(existing), Some(addr)) => {
                let mut obj = existing.clone();
                if let Some(map) = obj.as_object_mut() {
                    map.insert("token_address".to_string(), serde_json::json!(addr));
                }
                Some(obj)
            }
            (None, Some(addr)) => Some(serde_json::json!({ "token_address": addr })),
            (extra, None) => extra.clone(),
        };

        // Use ON CONFLICT (tx_hash, network) to handle duplicate PaymentDetected events
        // (e.g., after service restart). This is the unique constraint in the DB.
        sqlx::query(
            r#"
            INSERT INTO payments (
                id, invoice_id, network, asset_type, amount_value, asset_symbol,
                tx_hash, block_number, detected_at, confirmed_at, from_address, extra
            ) VALUES (
                $1, $2, $3::network, $4::asset_type, $5::numeric, $6,
                $7, $8, $9, $10, $11, $12
            )
            ON CONFLICT (tx_hash, network) DO UPDATE SET
                block_number = COALESCE(EXCLUDED.block_number, payments.block_number),
                confirmed_at = COALESCE(EXCLUDED.confirmed_at, payments.confirmed_at),
                extra = COALESCE(EXCLUDED.extra, payments.extra)
            "#,
        )
        .bind(payment.id)
        .bind(payment.invoice_id.as_str())
        .bind(try_network_to_db(payment.network)?)
        .bind(asset_type_to_db(payment.asset_type))
        .bind(&payment.amount)
        .bind(&payment.asset_symbol)
        .bind(&payment.tx_hash)
        .bind(payment.block_number.map(|n| n as i64))
        .bind(payment.detected_at)
        .bind(payment.confirmed_at)
        .bind(&payment.from_address)
        .bind(&extra)
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
        network: types::Network,
        fork_block: u64,
    ) -> RepositoryResult<u64> {
        let result = sqlx::query(
            r#"
            UPDATE payments
            SET reorged = TRUE, confirmed_at = NULL
            WHERE invoice_id = $1
              AND network = $2::network
              AND block_number >= $3
              AND reorged = FALSE
            "#,
        )
        .bind(invoice_id.as_str())
        .bind(try_network_to_db(network)?)
        .bind(fork_block as i64)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(result.rows_affected())
    }
}

/// Convert a database row to PaymentData.
fn try_row_to_payment(row: &sqlx::postgres::PgRow) -> RepositoryResult<PaymentData> {
    let block_number: Option<i64> = row.get("block_number");
    let asset_type_str: String = row.get("asset_type");
    let extra: Option<serde_json::Value> = row.get("extra");

    // Extract token_address from extra if present
    let token_address = extra.as_ref().and_then(|e| {
        e.get("token_address")
            .and_then(|v| v.as_str())
            .map(String::from)
    });

    Ok(PaymentData {
        id: row.get("id"),
        invoice_id: InvoiceId::from_string(row.get("invoice_id")),
        network: try_db_to_network(row.get("network"))?,
        asset_type: db_to_asset_type(&asset_type_str),
        amount: row.get("amount_value"),
        asset_symbol: row.get("asset_symbol"),
        token_address,
        tx_hash: row.get("tx_hash"),
        block_number: block_number.map(|n| n as u64),
        detected_at: row.get("detected_at"),
        confirmed_at: row.get("confirmed_at"),
        from_address: row.get("from_address"),
        reorged: row.get("reorged"),
        extra,
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
