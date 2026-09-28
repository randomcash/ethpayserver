//! Comparing the monitor's actual Redis watch set against what Postgres says
//! should be watched.
//!
//! `deletion.rs` names the failure this exists to catch: a no-TTL Redis key
//! pointing at an invoice id that no longer exists, left behind because the
//! unwatch step that follows a delete or a cleanup is best-effort by design.
//! Nothing in Postgres can see that - the cascade keeps Postgres itself
//! consistent, it just does not tell Redis. This is the other half: fetch
//! both sides and diff them.
//!
//! Detection only. Whether a mismatch found here should also be *cleared* is
//! a separate decision - an automatic unwatch is a side effect with no
//! rollback, the same reasoning that keeps `unwatch_after_delete` best-effort
//! and after-the-fact rather than eager.

use data_service::{PgDataService, reconcile};

use super::EVMMonitor;

/// Counts from comparing the expected watch set against the actual one. The
/// two directions are different faults - see `data_service::WatchReconciliation`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WatchReconciliationCounts {
    /// Watched in Redis, absent from the expected set - a deleted or
    /// resolved invoice still being polled.
    pub stale: usize,
    /// In the expected set, not watched in Redis - a live invoice nobody is
    /// watching. The more expensive fault: a real payment to it would go
    /// uncredited.
    pub missed: usize,
}

/// Error surfaced while gathering either side of the comparison. Deliberately
/// not merged with `EVMMonitorError` or `RepositoryError` - this is a
/// composition of both, not an extension of either.
#[derive(Debug, thiserror::Error)]
pub enum WatchReconciliationError {
    #[error("could not read the expected watch set from Postgres: {0}")]
    Expected(#[from] data_service::RepositoryError),

    #[error("could not read the actual watch set: {0}")]
    Actual(#[source] super::EVMMonitorError),
}

/// Fetch both sides and diff them.
pub async fn reconcile_watches(
    data_service: &PgDataService,
    monitor: &dyn EVMMonitor,
) -> Result<WatchReconciliationCounts, WatchReconciliationError> {
    let expected: Vec<_> = data_service
        .get_expected_watched_addresses()
        .await?
        .iter()
        .map(data_service::ExpectedWatch::key)
        .collect();

    let actual: Vec<_> = monitor
        .get_watched_addresses()
        .await
        .map_err(WatchReconciliationError::Actual)?
        .into_iter()
        .map(|(address, _invoice_id, chain_id, token_address)| {
            data_service::WatchKey::new(chain_id, &address, token_address.as_deref())
        })
        .collect();

    let diff = reconcile(&expected, &actual);
    Ok(WatchReconciliationCounts {
        stale: diff.stale.len(),
        missed: diff.missed.len(),
    })
}
