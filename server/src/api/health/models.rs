//! Response and data types for the health check endpoints.

use std::collections::HashMap;

use evm::monitor::{ChainHealth, SourceStatus};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Health check response.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    /// Service status.
    pub status: String,

    /// Service version.
    pub version: String,

    /// Build commit SHA (short) baked in at compile time.
    pub build_sha: String,

    /// Database connectivity.
    pub database: bool,

    /// Redis connectivity (for monitor service).
    /// None if Redis is not configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redis: Option<bool>,
}

/// Readiness probe response (returned on 503).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReadinessResponse {
    /// "ready" or "not_ready".
    pub status: String,
    /// Names of failing dependencies (e.g. "postgres", "redis", "rpc:56").
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failing: Vec<String>,
}

/// Status of a single dependency in the deep health check.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DependencyHealth {
    /// "ok" or "error".
    pub status: String,
    /// Latency in milliseconds.
    pub latency_ms: u64,
    /// Error message if status is "error".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// RPC chain status in the deep health check.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RpcHealth {
    /// "ok" or "error".
    pub status: String,
    /// Latency in milliseconds (time to read health from Redis, not RPC RTT).
    pub latency_ms: u64,
    /// Last block number reported by the monitor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_block: Option<u64>,
    /// Error message if status is "error".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Monitor liveness status in the deep health check.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MonitorHealth {
    /// "ok" or "error".
    pub status: String,
    /// Whether chain health data in Redis is fresh (updated within 60 s).
    pub data_fresh: bool,
}

/// Deep health diagnostic response.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DeepHealthResponse {
    /// Build commit SHA (short) baked in at compile time.
    pub build_sha: String,
    /// Service version from Cargo.toml.
    pub version: String,
    /// Postgres health.
    pub postgres: DependencyHealth,
    /// Redis health.
    pub redis: DependencyHealth,
    /// Per-chain RPC health keyed by chain_id.
    pub rpcs: HashMap<String, RpcHealth>,
    /// EVM monitor liveness.
    pub monitor: MonitorHealth,
}

/// Chain health information for a single chain.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ChainHealthInfo {
    /// Chain ID (EIP-155).
    pub chain_id: u64,
    /// Human-readable chain name.
    pub chain_name: String,
    /// Connection status (connected, connecting, disconnected, failed).
    pub status: String,
    /// Current block number on chain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_block: Option<u64>,
    /// Last block processed by the monitor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_processed_block: Option<u64>,
    /// Number of addresses being watched.
    pub watched_addresses: usize,
    /// Overall health status.
    pub is_healthy: bool,
}

impl From<ChainHealth> for ChainHealthInfo {
    fn from(h: ChainHealth) -> Self {
        Self {
            chain_id: h.chain_id,
            chain_name: h.chain_name,
            status: match h.status {
                SourceStatus::Connected => "connected".to_string(),
                SourceStatus::Connecting => "connecting".to_string(),
                SourceStatus::Disconnected => "disconnected".to_string(),
                SourceStatus::Failed(msg) => format!("failed: {}", msg),
            },
            current_block: h.current_block,
            last_processed_block: h.last_processed_block,
            watched_addresses: h.watched_addresses,
            is_healthy: h.is_healthy,
        }
    }
}

/// Chains health response.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ChainsHealthResponse {
    /// Health information for each monitored chain.
    pub chains: Vec<ChainHealthInfo>,
    /// Whether all chains are healthy.
    pub all_healthy: bool,
    /// Data freshness - whether health data is recent (updated within 60s).
    pub data_fresh: bool,
}
