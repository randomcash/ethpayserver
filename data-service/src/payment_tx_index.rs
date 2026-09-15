//! Upsert a payment keyed by transfer, not just transaction.
//!
//! One EVM transaction can pay multiple watched addresses - a batching
//! contract, a multicall, an exchange sweep. `payserver-commons::PaymentData`
//! has no field for which transfer within a transaction a payment came from:
//! it is a shared contract pinned by revision, and extending it needs a
//! three-repo merge dance (land in commons, bump the `rev` here, `cargo
//! update`) that a fix for a live data-loss bug should not have to wait on.
//! So the distinguishing value - the EVM monitor's `log_index` - travels as a
//! plain parameter instead of a struct field.
//!
//! Lives here rather than in `payserver-commons` for the same reason
//! `PaymentAnalyticsReader` does: not part of the payment-server contract
//! every backend has to satisfy.

use async_trait::async_trait;
use types::{PaymentData, RepositoryResult};

/// Upsert a payment keyed by `(chain_id, tx_hash, tx_index)` rather than
/// `(chain_id, tx_hash)` alone, so two transfers batched into one transaction
/// don't collide and silently overwrite each other.
#[async_trait]
pub trait PaymentTxIndexWriter: Send + Sync {
    async fn upsert_with_tx_index(
        &self,
        payment: &PaymentData,
        tx_index: i32,
    ) -> RepositoryResult<()>;
}

/// Read a payment back by the transfer it came from.
///
/// The confirmation handler used to find its row with
/// `payments.iter().find(|p| p.tx_hash == tx_hash)`, which was well defined
/// only while `unique_payment_tx` guaranteed one row per `(chain_id,
/// tx_hash)`. Once two transfers in one transaction each get a row, "the first
/// one with this hash" confirms an arbitrary one of them and leaves the other
/// unconfirmed for good - `mark_confirmed` is a no-op once set, so a repeat
/// event does not rescue it.
///
/// `PaymentData` carries no `tx_index` field, for the reason the writer above
/// describes, so the selection happens in SQL rather than by filtering rows in
/// the caller.
#[async_trait]
pub trait PaymentTxIndexReader: Send + Sync {
    /// The payment for one specific transfer, or `None` if no row matches.
    async fn get_by_tx_index(
        &self,
        invoice_id: &types::InvoiceId,
        tx_hash: &str,
        tx_index: i32,
    ) -> RepositoryResult<Option<PaymentData>>;
}
