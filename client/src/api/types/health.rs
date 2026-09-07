//! Health-related API types.

use serde::{Deserialize, Serialize};

/// Health of a single chain the monitor is watching.
///
/// Mirrors `ChainHealthInfo` from `server/src/api/health/models.rs`, which is
/// built from what evmmonitor publishes to Redis under `evmmonitor:health`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainHealthInfo {
    /// Chain ID (EIP-155).
    pub chain_id: u64,
    /// Human-readable chain name as the monitor reports it.
    pub chain_name: String,
    /// Connection status: "connected", "connecting", "disconnected" or
    /// "failed". Admins additionally get "failed: {reason}" — the reason is
    /// withheld from everyone else because an RPC error string routinely
    /// carries the provider host and its API key.
    pub status: String,
    /// Current head block on chain, when the monitor knows it.
    #[serde(default)]
    pub current_block: Option<u64>,
    /// Last block the monitor actually processed.
    #[serde(default)]
    pub last_processed_block: Option<u64>,
    /// Number of addresses being watched on this chain. Admin only, so absent
    /// for everyone else - `None` rather than `0`, because "not told" and
    /// "watching nothing" are different answers.
    #[serde(default)]
    pub watched_addresses: Option<usize>,
    /// The monitor's own verdict on this chain.
    #[serde(default)]
    pub is_healthy: bool,
}

/// Response from `GET /health/chains`.
///
/// Mirrors `ChainsHealthResponse` from the server API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainsHealthResponse {
    /// One entry per chain the monitor reported. Empty means the monitor has
    /// published nothing — not that every chain is fine.
    #[serde(default)]
    pub chains: Vec<ChainHealthInfo>,
    /// Whether every reported chain is healthy.
    #[serde(default)]
    pub all_healthy: bool,
    /// Whether the health data is recent. The Redis keys carry a 60s TTL, so
    /// `false` means the monitor stopped publishing.
    #[serde(default)]
    pub data_fresh: bool,
}
