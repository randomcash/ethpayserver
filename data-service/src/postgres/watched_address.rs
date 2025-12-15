//! Watched address repository implementation.

use async_trait::async_trait;
use chrono::Utc;
use sqlx::Row;

use crate::{RepositoryError, RepositoryResult, WatchedAddressReader, WatchedAddressWriter, sqlx_to_repo_error};
use types::{InvoiceId, Network};

use super::conversions::{try_db_to_network, try_network_to_db};
use super::PgDataService;

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
        .bind(try_network_to_db(network)?)
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

        let mut result = Vec::with_capacity(rows.len());
        for r in &rows {
            let address: String = r.get("address");
            let invoice_id = InvoiceId::from_string(r.get("invoice_id"));
            let network = try_db_to_network(r.get("network"))?;
            result.push((address, invoice_id, network));
        }
        Ok(result)
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

        let expires_at = invoice_row
            .map(|r| r.get("expires_at"))
            .unwrap_or_else(|| Utc::now() + chrono::Duration::hours(24));

        let network_db = try_network_to_db(network)?;

        // Use a transaction to handle the race condition between check and insert/update.
        // PostgreSQL's UNIQUE constraint doesn't match NULLs, so ON CONFLICT won't work
        // for (address, network, token_address) when token_address IS NULL.
        // We use a transaction with SELECT FOR UPDATE to prevent races.
        let mut tx = self.pool.begin().await.map_err(sqlx_to_repo_error)?;

        let existing = sqlx::query(
            r#"
            SELECT id FROM watched_addresses
            WHERE address = $1 AND network = $2::network AND token_address IS NULL
            FOR UPDATE
            "#,
        )
        .bind(address)
        .bind(network_db)
        .fetch_optional(&mut *tx)
        .await
        .map_err(sqlx_to_repo_error)?;

        if existing.is_some() {
            // Update existing row
            sqlx::query(
                r#"
                UPDATE watched_addresses
                SET invoice_id = $1, is_active = TRUE, expires_at = $2
                WHERE address = $3 AND network = $4::network AND token_address IS NULL
                "#,
            )
            .bind(invoice_id.as_str())
            .bind(expires_at)
            .bind(address)
            .bind(network_db)
            .execute(&mut *tx)
            .await
            .map_err(sqlx_to_repo_error)?;
        } else {
            // Insert new row. The SELECT FOR UPDATE above prevents race conditions.
            sqlx::query(
                r#"
                INSERT INTO watched_addresses (
                    address, invoice_id, network, asset_type, is_active, expires_at
                ) VALUES (
                    $1, $2, $3::network, 'native'::asset_type, TRUE, $4
                )
                "#,
            )
            .bind(address)
            .bind(invoice_id.as_str())
            .bind(network_db)
            .bind(expires_at)
            .execute(&mut *tx)
            .await
            .map_err(sqlx_to_repo_error)?;
        }

        tx.commit().await.map_err(sqlx_to_repo_error)?;

        Ok(())
    }

    async fn remove(&self, address: &str, network: Network) -> RepositoryResult<()> {
        let result = sqlx::query(
            r#"
            UPDATE watched_addresses
            SET is_active = FALSE
            WHERE address = $1 AND network = $2::network
            "#,
        )
        .bind(address)
        .bind(try_network_to_db(network)?)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound(format!(
                "Watched address not found: {} on {:?}",
                address, network
            )));
        }

        Ok(())
    }
}
