//! Payment analytics reads.
//!
//! These live here rather than in `payserver-commons` because they are a
//! dashboard concern, not part of the payment-server contract every backend
//! has to satisfy.
//!
//! The trait deliberately returns *raw* smallest-unit sums plus the decimals
//! they were denominated in, and leaves scaling to the caller. Two reasons:
//!
//! 1. `data-service` has no fixed-point decimal dependency, and doing the
//!    division in SQL on one backend and in Rust in the in-memory double is
//!    exactly the mock/store divergence that has bitten before.
//! 2. The same `asset_symbol` can arrive with different `decimals` (a token
//!    listed with the wrong decimals on one chain, a payment whose
//!    `payment_option` row was deleted). Keeping `decimals` in the group key
//!    means those rows stay separable instead of being silently summed as if
//!    they shared a unit.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use types::{RepositoryResult, StoreId};

/// A bounded request for per-day, per-asset payment volume.
#[derive(Debug, Clone)]
pub struct PaymentVolumeQuery {
    /// Stores to aggregate over.
    ///
    /// An EMPTY vec means "no stores" and MUST produce an empty result. It is
    /// not "every store": collapsing the two is how a user who belongs to no
    /// store ends up reading the whole server. There is deliberately no
    /// `None`/"all stores" variant here — the
    /// caller always names the stores it is entitled to.
    pub store_ids: Vec<StoreId>,

    /// Inclusive lower bound on `detected_at`. Callers derive it from a
    /// clamped day count so history is never aggregated unbounded.
    pub since: DateTime<Utc>,

    /// Exclusive upper bound on `detected_at`.
    pub until: DateTime<Utc>,
}

/// One `(UTC day, asset, decimals)` group of payments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentVolumeBucket {
    /// UTC calendar day the payments were detected on.
    pub day: NaiveDate,
    /// Asset symbol as recorded on the payment (e.g. `ETH`, `USDC`).
    pub asset_symbol: String,
    /// Decimals the summed amount is denominated in.
    pub decimals: u8,
    /// Sum of `payments.amount` in smallest units, as a decimal integer
    /// string — the column is `numeric(78, 0)`, wider than any Rust integer.
    pub raw_amount: String,
    /// Number of payments in the group.
    pub payment_count: i64,
}

/// Aggregate reads over payments, for dashboard analytics.
#[async_trait]
pub trait PaymentAnalyticsReader: Send + Sync {
    /// Sum non-reorged payments per UTC day and asset over a bounded window.
    ///
    /// Reorged payments are excluded: they were rolled back by the chain, so
    /// charting them would show a merchant money they never received.
    ///
    /// Days with no payments are simply absent — the caller decides how to
    /// present a gap. Ordering is `(day, asset_symbol, decimals)` ascending.
    async fn payment_volume_by_day(
        &self,
        query: &PaymentVolumeQuery,
    ) -> RepositoryResult<Vec<PaymentVolumeBucket>>;
}
