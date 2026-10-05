//! Finding and retracting payments invalidated by a chain reorganization.
//!
//! `PaymentWriter::mark_reorged` (from `types`) retracts every non-reorged
//! payment for one invoice at or above a fork block in a single statement.
//! That is exactly the right tool once the affected invoice is known, but
//! nothing in the shared trait can answer "which invoices does this chain's
//! reorg touch" — that question has no invoice_id yet, and the monitor's own
//! in-memory pending-payment map cannot answer it either: it is empty after a
//! restart and never contains a payment that has already confirmed.
//!
//! It also cannot answer "was this specific payment actually invalidated, or
//! did the transaction just move to a different block" — retracting a
//! payment that survived the reorg elsewhere un-pays an invoice that is
//! still genuinely paid, which is the opposite error.
//!
//! Lives here rather than in the shared `types` repository traits because it
//! is this server's rule, over this server's schema.

use async_trait::async_trait;
use types::{ChainId, PaymentData, RepositoryResult};
use uuid::Uuid;

/// Finds the durable candidate set for a chain reorganization.
#[async_trait]
pub trait ReorgCandidateReader: Send + Sync {
    /// Non-reorged payments on `chain_id` at or above `fork_block`.
    ///
    /// Unlike the monitor's own in-memory pending-payment map, this survives
    /// a monitor restart and still finds a payment that has since confirmed
    /// and dropped out of that map.
    async fn reorg_candidates(
        &self,
        chain_id: &ChainId,
        fork_block: u64,
    ) -> RepositoryResult<Vec<PaymentData>>;
}

/// Retracts a single payment invalidated by a chain reorganization.
#[async_trait]
pub trait ReorgWriter: Send + Sync {
    /// Mark one payment as reorged by id, undoing any confirmation.
    ///
    /// `PaymentWriter::mark_reorged` targets an invoice, chain, and fork
    /// block, retracting every matching payment in one statement. That is
    /// too coarse once a transaction has been re-validated as still present
    /// on chain in a different block: this targets exactly one payment, so
    /// that one can be left alone.
    async fn mark_payment_reorged(&self, id: Uuid) -> RepositoryResult<()>;
}
