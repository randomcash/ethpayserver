//! Response and data types for the health check endpoints.

pub use api_types::{
    ChainHealthInfo, ChainsHealthResponse, DeepHealthResponse, DependencyHealth, HealthResponse,
    MonitorHealth, ReadinessResponse, RpcHealth,
};
use evm::monitor::{ChainHealth, SourceStatus};

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
