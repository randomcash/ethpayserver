//! Detecting disagreement between the watches the database expects to be held
//! and the watches the monitor actually holds.
//!
//! Nothing else compares the two, and the two directions of disagreement cost
//! very different amounts:
//!
//! - **stale**: held by the monitor, not expected by the database. Wasteful
//!   only - a key that will never be paid into, scanned forever.
//! - **missing**: expected by the database, not held by the monitor. **Nobody
//!   is watching an address an invoice expects payment on**, so a payment can
//!   arrive and never be credited. This is the expensive one, and it is why
//!   the two are never added together into one number.
//!
//! The monitor side is read through [`crate::LiveWatchedAddressReader`], whose
//! live-watch store is the monitor's own durable record: it is what the monitor
//! rehydrates from on restart, and it is written by the monitor as it accepts
//! watch commands. The monitor's in-memory map is deliberately *not*
//! the comparison target - a component reporting on itself cannot detect that
//! it never received a command.
//!
//! Detection only. Nothing here clears or repairs a watch: unwatching is an
//! external side effect with no rollback, and a reconciler that acts on a
//! comparison it may have got wrong can stop watching an address that is about
//! to be paid into.
//!
//! # Known transient
//!
//! Cleanup unwatches before it deactivates the row (see the invoice cleanup
//! service), so between those two steps a watch reads as `missing` while the
//! system is behaving correctly. Such an entry belongs to an invoice that is
//! no longer pending, which is how it is told apart from the case that loses
//! money.

use std::collections::BTreeSet;

use sqlx::Row;

use crate::{LiveWatchedAddressReader, RepositoryResult, sqlx_to_repo_error};

use super::PgDataService;
use super::conversions::chain_id_from_row;

/// One watch, identified by the whole tuple that distinguishes it.
///
/// Not the address alone. The same address is watched separately per asset -
/// natively and once per token contract - so two watches on one address are
/// two independent entries here. Comparing on address alone would report an
/// address whose token watch had been lost as present and correct, because its
/// native watch still is; that is precisely the direction in which a payment
/// arrives uncredited.
///
/// `address` and `token_address` are held lower-cased so that a checksummed
/// database row and the monitor's lower-cased key compare equal instead of
/// producing a stale/missing pair for the same watch.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WatchKey {
    /// Payment address being watched, lower-cased.
    pub address: String,
    /// Invoice the watch belongs to.
    pub invoice_id: String,
    /// Chain the address is watched on.
    pub chain_id: types::ChainId,
    /// Token contract for an ERC20 watch, `None` for the native asset.
    pub token_address: Option<String>,
}

impl WatchKey {
    /// Build a key, normalising the case of the address and token contract.
    pub fn new(
        address: &str,
        invoice_id: &str,
        chain_id: types::ChainId,
        token_address: Option<&str>,
    ) -> Self {
        Self {
            address: address.to_lowercase(),
            invoice_id: invoice_id.to_string(),
            chain_id,
            token_address: token_address.map(str::to_lowercase),
        }
    }
}

/// The two directions of disagreement, kept apart.
///
/// There is no combined count and no single "is it healthy" number on purpose:
/// a caller that reports one total cannot tell a wasted key from an unwatched
/// invoice, and those warrant different responses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchDivergence {
    /// Held by the monitor, not expected by the database. Wasteful only.
    pub stale: Vec<WatchKey>,
    /// Expected by the database, not held by the monitor. A payment to one of
    /// these arrives and is credited to nobody.
    pub missing: Vec<WatchKey>,
}

impl WatchDivergence {
    /// How many watches the monitor holds that the database does not expect.
    pub fn stale_count(&self) -> usize {
        self.stale.len()
    }

    /// How many watches the database expects that the monitor does not hold.
    pub fn missing_count(&self) -> usize {
        self.missing.len()
    }

    /// Whether the two sides agree exactly.
    pub fn agrees(&self) -> bool {
        self.stale.is_empty() && self.missing.is_empty()
    }
}

/// Diff an expected set against a live set on the full tuple.
///
/// Both sides are de-duplicated and ordered, so the report is stable between
/// runs over the same data rather than following the live store's scan order.
pub fn compare_watches<E, L>(expected: E, live: L) -> WatchDivergence
where
    E: IntoIterator<Item = WatchKey>,
    L: IntoIterator<Item = WatchKey>,
{
    let expected: BTreeSet<WatchKey> = expected.into_iter().collect();
    let live: BTreeSet<WatchKey> = live.into_iter().collect();

    WatchDivergence {
        stale: live.difference(&expected).cloned().collect(),
        missing: expected.difference(&live).cloned().collect(),
    }
}

/// Compare what the database expects against what the monitor actually holds.
///
/// `monitor` is read first-hand rather than being asked for a summary, so the
/// comparison does not depend on the monitor agreeing that anything is wrong.
pub async fn reconcile_watches<E, M>(expected: E, monitor: &M) -> RepositoryResult<WatchDivergence>
where
    E: IntoIterator<Item = WatchKey>,
    M: LiveWatchedAddressReader + ?Sized,
{
    let live = monitor.get_all_watched().await?.into_iter().map(
        |(address, invoice_id, chain_id, token_address)| {
            WatchKey::new(
                &address,
                invoice_id.as_str(),
                chain_id,
                token_address.as_deref(),
            )
        },
    );

    Ok(compare_watches(expected, live))
}

impl PgDataService {
    /// Every watch the database still expects the monitor to be holding.
    ///
    /// Scoped by `is_active` and nothing else. `is_active` is the flag the
    /// system itself clears to record "this address is no longer watched", so
    /// it is the database's own statement of what should be watched; narrowing
    /// it further - to particular invoice statuses, say - would invent a policy
    /// no other query here applies, and a narrower expected set hides missing
    /// watches, which is the direction that loses a payment. The same
    /// unscoped-by-status reasoning the store-deletion query is built on.
    ///
    /// Joined through `payment_options` to `invoices` because the invoice is
    /// part of the comparison key and the monitor stores it as the watch's
    /// value: a row whose invoice has gone is not a watch anyone is waiting on.
    pub async fn get_expected_watches(&self) -> RepositoryResult<Vec<WatchKey>> {
        let rows = sqlx::query(
            r#"
            SELECT wa.address, wa.chain_id, wa.token_address, po.invoice_id
            FROM watched_addresses wa
            JOIN payment_options po ON wa.payment_option_id = po.id
            JOIN invoices i ON po.invoice_id = i.id
            WHERE wa.is_active = TRUE
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        let mut result = Vec::with_capacity(rows.len());
        for r in &rows {
            let address: String = r.get("address");
            let invoice_id: String = r.get("invoice_id");
            let token_address: Option<String> = r.get("token_address");
            result.push(WatchKey::new(
                &address,
                &invoice_id,
                chain_id_from_row(r, "chain_id"),
                token_address.as_deref(),
            ));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
