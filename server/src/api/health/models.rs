//! Response and data types for the health check endpoints.

pub use api_types::{
    ChainHealthInfo, ChainsHealthResponse, DeepHealthResponse, DependencyHealth, HealthResponse,
    MonitorHealth, ReadinessResponse, RpcHealth,
};
use evm::monitor::{ChainHealth, SourceStatus};

/// Comparison between the monitor's actual Redis watch set and what
/// Postgres's `expected_watched_addresses` view says should be watched.
///
/// Not part of `api_types::DeepHealthResponse`: that type is pinned by
/// revision in `payserver-commons`, and a new field there needs its own
/// merge-then-bump-pin cycle before this repo can see it. `deep_health`
/// serializes this alongside the shared response's fields instead of
/// waiting on that cycle - see its handler for how.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct WatchReconciliationHealth {
    /// "ok" once the comparison ran, "unknown" if it could not - no monitor
    /// configured, or the comparison itself failed or timed out.
    pub status: String,
    /// Watched in Redis, absent from the expected set: a deleted or
    /// resolved invoice the monitor is still polling.
    pub stale_watches: usize,
    /// In the expected set, not watched in Redis: a live invoice nobody is
    /// watching. Worse than a stale watch - a real payment to it would go
    /// uncredited.
    pub missed_watches: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl WatchReconciliationHealth {
    pub(super) fn unknown(reason: impl Into<String>) -> Self {
        Self {
            status: "unknown".to_string(),
            stale_watches: 0,
            missed_watches: 0,
            error: Some(reason.into()),
        }
    }
}

/// Build the wire shape from the monitor's per-chain health.
///
/// A free function rather than a `From` impl: `ChainHealth` belongs to `evm`
/// and `ChainHealthInfo` to `api-types`, so neither is local here and the
/// orphan rule forbids the impl. That is the rule working - the conversion is
/// EVM-specific and does not belong in a contract every chain shares.
pub(crate) fn chain_health_info(h: ChainHealth) -> ChainHealthInfo {
    ChainHealthInfo {
        // `ChainHealth` comes from the EVM monitor and carries an EIP-155
        // number. `.to_string()` on it yields "1", not "eip155:1" - which
        // the client now parses as a `ChainId` and rejects, taking the
        // whole chains-health response down with it.
        chain_id: types::ChainId::evm(h.chain_id),
        chain_name: h.chain_name,
        status: match h.status {
            SourceStatus::Connected => "connected".to_string(),
            SourceStatus::Connecting => "connecting".to_string(),
            SourceStatus::Disconnected => "disconnected".to_string(),
            SourceStatus::Failed(msg) => format!("failed: {}", msg),
        },
        current_block: h.current_block,
        last_processed_block: h.last_processed_block,
        watched_addresses: Some(h.watched_addresses),
        is_healthy: h.is_healthy,
    }
}
