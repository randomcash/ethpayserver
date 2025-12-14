//! PostgreSQL implementation of the repository traits.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::Row;
use sqlx::postgres::PgPool;
use uuid::Uuid;

use crate::{
    InvoiceQueryParams, InvoiceReader, InvoiceWriter, PaymentReader, PaymentWriter,
    RepositoryResult, WatchedAddressReader, WatchedAddressWriter, sqlx_to_repo_error,
};
use types::{InvoiceData, InvoiceId, InvoiceStatus, Network, PaymentData};

/// PostgreSQL data service implementation.
#[derive(Clone)]
pub struct PgDataService {
    pool: PgPool,
}

impl PgDataService {
    /// Create a new PgDataService with the given connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

// =============================================================================
// Helper functions for enum conversions
// =============================================================================

/// Convert Rust Network enum to database string.
fn network_to_db(network: Network) -> &'static str {
    match network {
        Network::Ethereum => "ethereum",
        Network::Polygon => "polygon",
        Network::Arbitrum => "arbitrum",
        Network::Optimism => "optimism",
        Network::Base => "base",
        Network::Avalanche => "avalanche",
        Network::BinanceSmartChain => "binance_smart_chain",
        Network::ZkSync => "zksync",
        Network::Linea => "linea",
        Network::Scroll => "scroll",
        // All other networks are not supported, falling back to Ethereum
        _ => "ethereum", // fallback, shouldn't happen
    }
}

/// Convert database string to Rust Network enum.
fn db_to_network(s: &str) -> Network {
    match s {
        "ethereum" => Network::Ethereum,
        "polygon" => Network::Polygon,
        "arbitrum" => Network::Arbitrum,
        "optimism" => Network::Optimism,
        "base" => Network::Base,
        "avalanche" => Network::Avalanche,
        "binance_smart_chain" => Network::BinanceSmartChain,
        "zksync" => Network::ZkSync,
        "linea" => Network::Linea,
        "scroll" => Network::Scroll,
        _ => Network::Ethereum, // fallback
    }
}

/// Convert Rust InvoiceStatus enum to database string.
fn status_to_db(status: InvoiceStatus) -> &'static str {
    match status {
        InvoiceStatus::Pending => "pending",
        InvoiceStatus::Processing => "processing",
        InvoiceStatus::PartiallyPaid => "partially_paid",
        InvoiceStatus::Paid => "paid",
        InvoiceStatus::Expired => "expired",
        InvoiceStatus::Cancelled => "cancelled",
        InvoiceStatus::Refunded => "refunded",
    }
}

/// Convert database string to Rust InvoiceStatus enum.
fn db_to_status(s: &str) -> InvoiceStatus {
    match s {
        "pending" => InvoiceStatus::Pending,
        "processing" => InvoiceStatus::Processing,
        "partially_paid" => InvoiceStatus::PartiallyPaid,
        "paid" => InvoiceStatus::Paid,
        "expired" => InvoiceStatus::Expired,
        "cancelled" => InvoiceStatus::Cancelled,
        "refunded" => InvoiceStatus::Refunded,
        _ => InvoiceStatus::Pending, // fallback
    }
}

// =============================================================================
// Invoice Repository Implementation
// =============================================================================

#[async_trait]
impl InvoiceReader for PgDataService {
    async fn get(&self, id: &InvoiceId) -> RepositoryResult<Option<InvoiceData>> {
        let row = sqlx::query(
            r#"
            SELECT
                id, network::text, status::text, amount_value::text,
                amount_received::text, asset_symbol, payment_address,
                created_at, expires_at, metadata, extra
            FROM invoices
            WHERE id = $1
            "#,
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row.map(|r| row_to_invoice(&r)))
    }

    async fn query(
        &self,
        params: &InvoiceQueryParams,
    ) -> RepositoryResult<(i64, Vec<InvoiceData>)> {
        // Build dynamic query for filtering
        let mut conditions = Vec::new();
        let mut bind_idx = 1;

        if params.status.is_some() {
            conditions.push(format!("status = ${}", bind_idx));
            bind_idx += 1;
        }
        if params.network.is_some() {
            conditions.push(format!("network = ${}", bind_idx));
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
                id, network::text, status::text, amount_value::text,
                amount_received::text, asset_symbol, payment_address,
                created_at, expires_at, metadata, extra
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
        if let Some(status) = params.status {
            count_query = count_query.bind(status_to_db(status));
        }
        if let Some(network) = params.network {
            count_query = count_query.bind(network_to_db(network));
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
        if let Some(status) = params.status {
            data_query = data_query.bind(status_to_db(status));
        }
        if let Some(network) = params.network {
            data_query = data_query.bind(network_to_db(network));
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

        let invoices: Vec<InvoiceData> = rows.iter().map(row_to_invoice).collect();

        Ok((total, invoices))
    }

    async fn get_expired(&self) -> RepositoryResult<Vec<InvoiceData>> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, network::text, status::text, amount_value::text,
                amount_received::text, asset_symbol, payment_address,
                created_at, expires_at, metadata, extra
            FROM invoices
            WHERE status IN ('pending', 'processing', 'partially_paid')
              AND expires_at < NOW()
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(rows.iter().map(row_to_invoice).collect())
    }
}

#[async_trait]
impl InvoiceWriter for PgDataService {
    async fn upsert(&self, invoice: &InvoiceData) -> RepositoryResult<()> {
        sqlx::query(
            r#"
            INSERT INTO invoices (
                id, network, status, amount_value, amount_received,
                asset_symbol, payment_address, created_at, expires_at,
                metadata, extra, asset_type
            ) VALUES (
                $1, $2::network, $3::invoice_status, $4::numeric, $5::numeric,
                $6, $7, $8, $9, $10, $11, 'native'::asset_type
            )
            ON CONFLICT (id) DO UPDATE SET
                status = EXCLUDED.status,
                amount_value = EXCLUDED.amount_value,
                amount_received = EXCLUDED.amount_received,
                asset_symbol = EXCLUDED.asset_symbol,
                payment_address = EXCLUDED.payment_address,
                expires_at = EXCLUDED.expires_at,
                metadata = EXCLUDED.metadata,
                extra = EXCLUDED.extra
            "#,
        )
        .bind(invoice.id.as_str())
        .bind(network_to_db(invoice.network))
        .bind(status_to_db(invoice.status))
        .bind(&invoice.amount)
        .bind(&invoice.amount_received)
        .bind(&invoice.asset_symbol)
        .bind(&invoice.payment_address)
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
        sqlx::query("UPDATE invoices SET status = $1::invoice_status WHERE id = $2")
            .bind(status_to_db(status))
            .bind(id.as_str())
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        Ok(())
    }

    async fn update_amount_received(&self, id: &InvoiceId, amount: &str) -> RepositoryResult<()> {
        sqlx::query("UPDATE invoices SET amount_received = $1::numeric WHERE id = $2")
            .bind(amount)
            .bind(id.as_str())
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        Ok(())
    }
}

/// Convert a database row to InvoiceData.
fn row_to_invoice(row: &sqlx::postgres::PgRow) -> InvoiceData {
    InvoiceData {
        id: InvoiceId::from_string(row.get("id")),
        network: db_to_network(row.get("network")),
        status: db_to_status(row.get("status")),
        amount: row.get("amount_value"),
        amount_received: row.get("amount_received"),
        asset_symbol: row.get("asset_symbol"),
        payment_address: row.get("payment_address"),
        payment_request: None, // Not stored in DB for EVM
        created_at: row.get("created_at"),
        expires_at: row.get("expires_at"),
        metadata: row.get("metadata"),
        extra: row.get("extra"),
    }
}

// =============================================================================
// Payment Repository Implementation
// =============================================================================

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

        Ok(row.map(|r| row_to_payment(&r)))
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

        Ok(rows.iter().map(row_to_payment).collect())
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

        Ok(rows.iter().map(row_to_payment).collect())
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
        .bind(network_to_db(payment.network))
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
        sqlx::query(
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

        Ok(())
    }
}

/// Convert a database row to PaymentData.
fn row_to_payment(row: &sqlx::postgres::PgRow) -> PaymentData {
    let block_number: Option<i64> = row.get("block_number");
    let confirmations: i32 = row.get("confirmations");

    PaymentData {
        id: row.get("id"),
        invoice_id: InvoiceId::from_string(row.get("invoice_id")),
        network: db_to_network(row.get("network")),
        amount: row.get("amount_value"),
        asset_symbol: row.get("asset_symbol"),
        tx_hash: row.get("tx_hash"),
        block_number: block_number.map(|n| n as u64),
        confirmations: confirmations as u32,
        detected_at: row.get("detected_at"),
        confirmed_at: row.get("confirmed_at"),
        from_address: row.get("from_address"),
        extra: row.get("extra"),
    }
}

// =============================================================================
// Watched Address Repository Implementation
// =============================================================================

#[async_trait]
impl WatchedAddressReader for PgDataService {
    async fn get_invoice_id(
        &self,
        address: &str,
        network: Network,
    ) -> RepositoryResult<Option<InvoiceId>> {
        let row = sqlx::query(
            r#"
            SELECT invoice_id
            FROM watched_addresses
            WHERE address = $1 AND network = $2::network AND is_active = TRUE
            "#,
        )
        .bind(address)
        .bind(network_to_db(network))
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row.map(|r| InvoiceId::from_string(r.get("invoice_id"))))
    }

    async fn get_active(&self) -> RepositoryResult<Vec<(String, InvoiceId, Network)>> {
        let rows = sqlx::query(
            r#"
            SELECT address, invoice_id, network::text
            FROM watched_addresses
            WHERE is_active = TRUE AND expires_at > NOW()
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(rows
            .iter()
            .map(|r| {
                let address: String = r.get("address");
                let invoice_id = InvoiceId::from_string(r.get("invoice_id"));
                let network = db_to_network(r.get("network"));
                (address, invoice_id, network)
            })
            .collect())
    }
}

#[async_trait]
impl WatchedAddressWriter for PgDataService {
    async fn upsert(
        &self,
        address: &str,
        invoice_id: &InvoiceId,
        network: Network,
    ) -> RepositoryResult<()> {
        // Get the invoice's expiration for the watched address
        let invoice_row = sqlx::query("SELECT expires_at FROM invoices WHERE id = $1")
            .bind(invoice_id.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        let expires_at: DateTime<Utc> = invoice_row
            .map(|r| r.get("expires_at"))
            .unwrap_or_else(|| Utc::now() + chrono::Duration::hours(24));

        sqlx::query(
            r#"
            INSERT INTO watched_addresses (
                address, invoice_id, network, asset_type, is_active, expires_at
            ) VALUES (
                $1, $2, $3::network, 'native'::asset_type, TRUE, $4
            )
            ON CONFLICT (address, network, token_address) DO UPDATE SET
                invoice_id = EXCLUDED.invoice_id,
                is_active = TRUE,
                expires_at = EXCLUDED.expires_at
            "#,
        )
        .bind(address)
        .bind(invoice_id.as_str())
        .bind(network_to_db(network))
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(())
    }

    async fn remove(&self, address: &str, network: Network) -> RepositoryResult<()> {
        sqlx::query(
            r#"
            UPDATE watched_addresses
            SET is_active = FALSE
            WHERE address = $1 AND network = $2::network
            "#,
        )
        .bind(address)
        .bind(network_to_db(network))
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_conversion() {
        assert_eq!(network_to_db(Network::Ethereum), "ethereum");
        assert_eq!(network_to_db(Network::Polygon), "polygon");
        assert_eq!(
            network_to_db(Network::BinanceSmartChain),
            "binance_smart_chain"
        );

        assert_eq!(db_to_network("ethereum"), Network::Ethereum);
        assert_eq!(db_to_network("polygon"), Network::Polygon);
        assert_eq!(
            db_to_network("binance_smart_chain"),
            Network::BinanceSmartChain
        );
    }

    #[test]
    fn test_status_conversion() {
        assert_eq!(status_to_db(InvoiceStatus::Pending), "pending");
        assert_eq!(status_to_db(InvoiceStatus::Paid), "paid");
        assert_eq!(status_to_db(InvoiceStatus::PartiallyPaid), "partially_paid");

        assert_eq!(db_to_status("pending"), InvoiceStatus::Pending);
        assert_eq!(db_to_status("paid"), InvoiceStatus::Paid);
        assert_eq!(db_to_status("partially_paid"), InvoiceStatus::PartiallyPaid);
    }
}
