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
