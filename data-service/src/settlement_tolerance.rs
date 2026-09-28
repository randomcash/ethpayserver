//! Per-store settlement tolerance and the audit of when it was used.
//!
//! An invoice settles when `amount_received >= amount - tolerance`, where the
//! tolerance is a percentage of the invoice amount, so one setting means the
//! same thing on a $5 invoice and a $50,000 one, in any invoice currency.
//!
//! Lives here rather than in `payserver-commons` for the same reason
//! `PaymentTxIndexReader` does: it is not part of the contract every backend
//! must satisfy, and extending the shared types needs a three-repo merge.
//! Decimals travel as strings so nothing rounds them on the way through.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use types::{InvoiceId, RepositoryResult};
use uuid::Uuid;

/// Tolerance, in percent of the invoice amount, for a store that has not set
/// one. Never zero: quotes are rounded to whole base units, so exact payment
/// can land a hair short and would otherwise never settle. Far below any real
/// underpayment (1e-6 of the invoice).
pub const DEFAULT_TOLERANCE_PERCENT: &str = "0.0001";

/// The most a store may set, in percent. Refused above, never clamped.
pub const MAX_TOLERANCE_PERCENT: &str = "1";

/// A shortfall that was accepted because of the tolerance.
#[derive(Debug, Clone, PartialEq)]
pub struct SettlementAllowance {
    pub invoice_id: String,
    /// Invoice amount minus amount received, in the invoice currency.
    pub shortfall: String,
    pub tolerance_percent: String,
    /// `"store"` when the store's own setting applied, `"default"` otherwise.
    pub source: String,
    pub recorded_at: DateTime<Utc>,
}

#[async_trait]
pub trait SettlementToleranceReader: Send + Sync {
    /// The store's own tolerance in percent, if it has set one.
    async fn get_settlement_tolerance(&self, store_id: Uuid) -> RepositoryResult<Option<String>>;

    async fn get_settlement_allowance(
        &self,
        invoice_id: &InvoiceId,
    ) -> RepositoryResult<Option<SettlementAllowance>>;
}

#[async_trait]
pub trait SettlementToleranceWriter: Send + Sync {
    async fn set_settlement_tolerance(
        &self,
        store_id: Uuid,
        tolerance_percent: &str,
    ) -> RepositoryResult<()>;

    /// Revert the store to the server default.
    async fn clear_settlement_tolerance(&self, store_id: Uuid) -> RepositoryResult<()>;

    /// Record the shortfall a tolerance accepted. Idempotent per invoice: the
    /// first record stands.
    async fn record_settlement_allowance(
        &self,
        invoice_id: &InvoiceId,
        shortfall: &str,
        tolerance_percent: &str,
        source: &str,
    ) -> RepositoryResult<()>;
}
