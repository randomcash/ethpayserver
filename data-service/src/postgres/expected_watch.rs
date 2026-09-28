//! What the monitor should currently be watching, per the
//! `expected_watched_addresses` view (see its migration for the exact
//! definition and why `is_active` alone is not enough).
//!
//! Not on `WatchedAddressReader`: that trait is defined in `types` and shared
//! across the workspace by revision pin, and this reconciliation need - the
//! Postgres side of comparing against the monitor's actual Redis watch set -
//! is local to this repo.

use sqlx::Row;

use crate::{RepositoryResult, WatchKey, sqlx_to_repo_error};
use types::InvoiceId;

use super::PgDataService;
use super::conversions::chain_id_from_row;

/// One row of the authoritative "should be watched" set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedWatch {
    pub address: String,
    pub chain_id: types::ChainId,
    pub token_address: Option<String>,
    pub invoice_id: InvoiceId,
}

impl ExpectedWatch {
    /// This row's identity for comparison against the monitor's actual watch
    /// set - see `WatchKey` for why the invoice id is not part of it.
    pub fn key(&self) -> WatchKey {
        WatchKey::new(
            self.chain_id.clone(),
            &self.address,
            self.token_address.as_deref(),
        )
    }
}

impl PgDataService {
    /// Every address that should currently be watched, per
    /// `expected_watched_addresses`.
    pub async fn get_expected_watched_addresses(&self) -> RepositoryResult<Vec<ExpectedWatch>> {
        let rows = sqlx::query(
            r#"
            SELECT address, chain_id, token_address, invoice_id
            FROM expected_watched_addresses
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(rows
            .iter()
            .map(|r| ExpectedWatch {
                address: r.get("address"),
                chain_id: chain_id_from_row(r, "chain_id"),
                token_address: r.get("token_address"),
                invoice_id: InvoiceId::from_string(r.get("invoice_id")),
            })
            .collect())
    }
}
