//! The one watched-address query account/store deletion needs.
//!
//! Split out of `watched_address.rs` rather than grown in it: that file is
//! the shared `WatchedAddressReader`/`WatchedAddressWriter` implementation,
//! scoped by invoice status for the background cleanup jobs that are its only
//! other callers. This query is deliberately unscoped by status, because
//! deleting an account or a store needs to know about every address still
//! watched for any invoice of theirs - including a `pending`, never-expired
//! one - and belongs with the deletion path that reads it, not the cleanup
//! trait none of its callers use.

use sqlx::Row;
use uuid::Uuid;

use crate::{CleanupAddressInfo, RepositoryResult, sqlx_to_repo_error};
use types::PaymentOptionId;

use super::PgDataService;
use super::conversions::chain_id_from_row;

impl PgDataService {
    /// Active watched addresses for invoices under the given stores,
    /// regardless of invoice status.
    ///
    /// Not on `WatchedAddressReader`: that trait's other queries are all
    /// scoped by invoice status (expired/paid/cancelled) because the
    /// background cleanup only ever wants those. Deleting a store or an
    /// account is different - every address still watched for any invoice of
    /// theirs has to go, including a `pending`, never-expired one, which is
    /// exactly the case the other queries would never surface: an invoice
    /// with an address generated and watched but no payment recorded yet.
    /// Adding it here keeps that store/account-deletion-only need out of the
    /// shared cross-repo trait.
    pub async fn get_active_watched_addresses_for_stores(
        &self,
        store_ids: &[Uuid],
    ) -> RepositoryResult<Vec<CleanupAddressInfo>> {
        if store_ids.is_empty() {
            return Ok(Vec::new());
        }

        let rows = sqlx::query(
            r#"
            SELECT wa.address, wa.payment_option_id, wa.chain_id, wa.token_address, po.invoice_id
            FROM watched_addresses wa
            JOIN payment_options po ON wa.payment_option_id = po.id
            JOIN invoices i ON po.invoice_id = i.id
            WHERE wa.is_active = TRUE
              AND i.store_id = ANY($1)
            "#,
        )
        .bind(store_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        let mut result = Vec::with_capacity(rows.len());
        for r in &rows {
            let payment_option_id: Uuid = r.get("payment_option_id");
            let chain_id = chain_id_from_row(r, "chain_id");
            result.push(CleanupAddressInfo {
                address: r.get("address"),
                payment_option_id: PaymentOptionId(payment_option_id),
                invoice_id: r.get("invoice_id"),
                chain_id,
                token_address: r.get("token_address"),
            });
        }
        Ok(result)
    }
}
