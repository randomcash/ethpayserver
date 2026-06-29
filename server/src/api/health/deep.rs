//! Deep health diagnostic endpoint and its dependency probes.

use std::collections::HashMap;

use axum::{Json, extract::State, http::StatusCode};
use evm::monitor::{ChainHealth, SourceStatus};
use tokio::time::Instant;

use crate::services::{EVMMonitor, RedisEVMMonitor};
use crate::state::PgAppState;

use super::PROBE_TIMEOUT;
use super::models::{DeepHealthResponse, DependencyHealth, MonitorHealth, RpcHealth};

/// Deep health diagnostic endpoint.
///
/// Returns per-dependency status with latencies. Not tied to load-balancer
/// decisions; intended for operators and dashboards. No authentication required.
#[utoipa::path(
    get,
    path = "/health/deep",
    tag = "health",
    responses(
        (status = 200, description = "Deep health diagnostic", body = DeepHealthResponse),
    )
)]
#[allow(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    reason = "deep health probe: timed Postgres + Redis + per-chain RPC checks, each with its own error mapping; splitting would obscure the HTTP response shape"
)]
pub async fn deep_health<A>(
    State(state): State<PgAppState<A>>,
) -> (StatusCode, Json<DeepHealthResponse>)
where
    A: Send + Sync + 'static,
{
    let postgres = probe_dependency(state.data_service.health_check()).await;

    let (redis, rpcs, monitor) = match state.evm_monitor.as_ref() {
        Some(evm_monitor) => probe_evm_monitor(evm_monitor).await,
        None => (
            DependencyHealth {
                status: "ok".to_string(),
                latency_ms: 0,
                error: Some("not configured".to_string()),
            },
            HashMap::new(),
            MonitorHealth {
                status: "ok".to_string(),
                data_fresh: false,
            },
        ),
    };

    (
        StatusCode::OK,
        Json(DeepHealthResponse {
            build_sha: env!("ETHPAYSERVER_BUILD_SHA").to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            postgres,
            redis,
            rpcs,
            monitor,
        }),
    )
}

/// Timeout a health-check future and convert the outcome into `DependencyHealth`.
async fn probe_dependency<F, T, E>(fut: F) -> DependencyHealth
where
    F: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let start = Instant::now();
    let result = tokio::time::timeout(PROBE_TIMEOUT, fut).await;
    let latency_ms = start.elapsed().as_millis() as u64;
    match result {
        Ok(Ok(_)) => DependencyHealth {
            status: "ok".to_string(),
            latency_ms,
            error: None,
        },
        Ok(Err(e)) => DependencyHealth {
            status: "error".to_string(),
            latency_ms,
            error: Some(e.to_string()),
        },
        Err(_) => DependencyHealth {
            status: "error".to_string(),
            latency_ms,
            error: Some("timeout".to_string()),
        },
    }
}

/// Probe EVM-monitor-backed dependencies (Redis health + per-chain RPC health +
/// monitor freshness) and assemble the three sub-sections of `DeepHealthResponse`.
async fn probe_evm_monitor(
    evm_monitor: &RedisEVMMonitor,
) -> (DependencyHealth, HashMap<String, RpcHealth>, MonitorHealth) {
    let redis = probe_dependency(evm_monitor.health_check()).await;

    let chains_start = Instant::now();
    let chains_result = tokio::time::timeout(PROBE_TIMEOUT, evm_monitor.get_chain_health()).await;
    let chains_latency = chains_start.elapsed().as_millis() as u64;

    let (rpcs, data_fresh) = match chains_result {
        Ok(Ok(chains)) => {
            let fresh = !chains.is_empty();
            (build_rpc_map(chains, chains_latency), fresh)
        }
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "deep health: failed to get chain health");
            (HashMap::new(), false)
        }
        Err(_) => {
            tracing::warn!("deep health: chain health fetch timed out");
            (HashMap::new(), false)
        }
    };

    let monitor = MonitorHealth {
        status: if data_fresh { "ok" } else { "error" }.to_string(),
        data_fresh,
    };

    (redis, rpcs, monitor)
}

fn build_rpc_map(chains: Vec<ChainHealth>, latency_ms: u64) -> HashMap<String, RpcHealth> {
    chains
        .into_iter()
        .map(|chain| {
            let status = if chain.is_healthy { "ok" } else { "error" };
            let error = if chain.is_healthy {
                None
            } else {
                Some(match &chain.status {
                    SourceStatus::Failed(msg) => msg.clone(),
                    SourceStatus::Disconnected => "disconnected".to_string(),
                    SourceStatus::Connecting => "connecting".to_string(),
                    _ => "unhealthy".to_string(),
                })
            };
            (
                chain.chain_id.to_string(),
                RpcHealth {
                    status: status.to_string(),
                    latency_ms,
                    last_block: chain.current_block,
                    error,
                },
            )
        })
        .collect()
}
