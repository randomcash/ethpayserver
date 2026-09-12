//! Which invoices a store's payouts have already claimed.
//!
//! A payout row is the only record that an invoice's money has been spoken for.
//! Nothing else marks an invoice as settled: there is no flag on the invoice, no
//! ledger, and no service that moves funds and writes back. So the question
//! "has this invoice already been paid out?" can only be answered by looking at
//! the payouts that name it.
//!
//! `PayoutReader` cannot answer it. `get_payouts_for_store` pages through every
//! payout a store has ever had and leaves the caller to unpack each
//! `invoice_ids` array in Rust — an unbounded scan on a request path, and one
//! that silently misses anything past the page it happened to ask for. This
//! trait asks the database the question directly instead.
//!
//! It lives here rather than in the shared `types` repository traits because it
//! is this server's rule, over this server's schema.

use async_trait::async_trait;
use types::{RepositoryResult, StoreId};

/// Reads which invoice ids a store's existing payouts already claim.
#[async_trait]
pub trait PayoutClaimReader: Send + Sync {
    /// Of `invoice_ids`, the ones already named by a payout of `store_id` that
    /// has not failed.
    ///
    /// Failed payouts release their invoices: a payout that failed moved no
    /// money, so the money is still there to claim. Pending, broadcasting and
    /// confirmed payouts all hold theirs — pending included, because a second
    /// payout raised while the first is still pending is exactly the double
    /// claim this exists to refuse.
    ///
    /// Scoped to the store on purpose. A caller has already been shown to own
    /// `store_id`; telling it about another store's payouts, even by refusing,
    /// would report on data it cannot see.
    async fn invoice_ids_already_claimed(
        &self,
        store_id: StoreId,
        invoice_ids: &[String],
    ) -> RepositoryResult<Vec<String>>;
}
