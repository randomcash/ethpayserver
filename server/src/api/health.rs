//! Health check endpoints.

use std::collections::HashMap;
use std::time::Duration;

use auth::SessionService;
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use evm::monitor::{ChainHealth, SourceStatus};
use serde::{Deserialize, Serialize};
use tokio::time::Instant;
use utoipa::ToSchema;

use crate::api::AdminAuth;
use crate::metrics;
use crate::services::{EVMMonitor, RedisEVMMonitor};
use crate::state::PgAppState;

/// Timeout for each individual dependency check in readiness/deep probes.
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);

/// Health check response.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    /// Service status.
    pub status: String,

    /// Service version.
    pub version: String,

    /// Database connectivity.
    pub database: bool,

    /// Redis connectivity (for monitor service).
    /// None if Redis is not configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redis: Option<bool>,
}

/// Health check endpoint.
///
/// Returns the service health status including database connectivity.
#[utoipa::path(
    get,
    path = "/health",
    tag = "health",
    responses(
        (status = 200, description = "Service is healthy", body = HealthResponse),
        (status = 503, description = "Service is unhealthy", body = HealthResponse),
    )
)]
pub async fn health_check<A>(
    State(state): State<PgAppState<A>>,
) -> (StatusCode, Json<HealthResponse>)
where
    A: Send + Sync + 'static,
{
    let db_healthy = state.data_service.health_check().await.is_ok();

    // Check Redis health if EVM monitor is configured
    let redis_healthy = if let Some(ref monitor) = state.evm_monitor {
        Some(monitor.health_check().await.is_ok())
    } else {
        None
    };

    // Service is healthy if DB is healthy (Redis is optional)
    let all_healthy = db_healthy && redis_healthy.unwrap_or(true);

    let response = HealthResponse {
        status: if all_healthy {
            "healthy".to_string()
        } else {
            "unhealthy".to_string()
        },
        version: env!("CARGO_PKG_VERSION").to_string(),
        database: db_healthy,
        redis: redis_healthy,
    };

    let status = if all_healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (status, Json(response))
}

/// Simple liveness probe.
///
/// Always returns 200 OK if the server is running.
#[utoipa::path(
    get,
    path = "/health/live",
    tag = "health",
    responses(
        (status = 200, description = "Service is alive"),
    )
)]
pub async fn liveness() -> StatusCode {
    StatusCode::OK
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

/// Readiness probe.
///
/// Returns 200 if the service is ready to accept traffic (Postgres, Redis,
/// and every configured RPC chain respond within 1 s). Returns 503 with a
/// JSON body listing the failing dependencies otherwise.
#[utoipa::path(
    get,
    path = "/health/ready",
    tag = "health",
    responses(
        (status = 200, description = "Service is ready", body = ReadinessResponse),
        (status = 503, description = "Service is not ready", body = ReadinessResponse),
    )
)]
pub async fn readiness<A>(
    State(state): State<PgAppState<A>>,
) -> (StatusCode, Json<ReadinessResponse>)
where
    A: Send + Sync + 'static,
{
    let mut failing = Vec::new();

    // Check Postgres with timeout
    let db_ok = tokio::time::timeout(PROBE_TIMEOUT, state.data_service.health_check())
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false);
    if !db_ok {
        failing.push("postgres".to_string());
    }

    // Check Redis + RPC chains if EVM monitor is configured
    if let Some(ref monitor) = state.evm_monitor {
        let redis_ok = tokio::time::timeout(PROBE_TIMEOUT, monitor.health_check())
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false);
        if !redis_ok {
            failing.push("redis".to_string());
        }

        // Check RPC chain health (read from Redis, published by evmmonitor)
        if redis_ok
            && let Ok(Ok(chains)) =
                tokio::time::timeout(PROBE_TIMEOUT, monitor.get_chain_health()).await
        {
            for chain in &chains {
                if !chain.is_healthy {
                    failing.push(format!("rpc:{}", chain.chain_id));
                }
            }
        }
    }

    if failing.is_empty() {
        (
            StatusCode::OK,
            Json(ReadinessResponse {
                status: "ready".to_string(),
                failing: vec![],
            }),
        )
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ReadinessResponse {
                status: "not_ready".to_string(),
                failing,
            }),
        )
    }
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
    /// Postgres health.
    pub postgres: DependencyHealth,
    /// Redis health.
    pub redis: DependencyHealth,
    /// Per-chain RPC health keyed by chain_id.
    pub rpcs: HashMap<String, RpcHealth>,
    /// EVM monitor liveness.
    pub monitor: MonitorHealth,
}

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

/// Get health status for all monitored chains.
///
/// Returns per-chain health information from the evmmonitor service.
/// Health data is published to Redis by evmmonitor every 10 seconds.
///
/// Requires server admin authentication.
#[utoipa::path(
    get,
    path = "/health/chains",
    tag = "health",
    responses(
        (status = 200, description = "Chain health information", body = ChainsHealthResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin access required"),
        (status = 503, description = "Health data unavailable"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn chains_health<A>(
    _admin: AdminAuth,
    State(state): State<PgAppState<A>>,
) -> (StatusCode, Json<ChainsHealthResponse>)
where
    A: SessionService + 'static,
{
    // Get Redis connection from EVM monitor
    let Some(ref monitor) = state.evm_monitor else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ChainsHealthResponse {
                chains: vec![],
                all_healthy: false,
                data_fresh: false,
            }),
        );
    };

    // Read health data from Redis
    match monitor.get_chain_health().await {
        Ok(chains) => {
            let all_healthy = chains.iter().all(|c| c.is_healthy);

            // Update watched addresses gauge for each chain
            for chain in &chains {
                metrics::set_watched_addresses(chain.chain_id, chain.watched_addresses);
            }

            let chain_infos: Vec<ChainHealthInfo> = chains.into_iter().map(Into::into).collect();

            (
                StatusCode::OK,
                Json(ChainsHealthResponse {
                    chains: chain_infos,
                    all_healthy,
                    data_fresh: true,
                }),
            )
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to get chain health from Redis");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ChainsHealthResponse {
                    chains: vec![],
                    all_healthy: false,
                    data_fresh: false,
                }),
            )
        }
    }
}

/// Prometheus metrics endpoint.
///
/// Returns metrics in Prometheus exposition format for scraping.
///
/// Requires server admin authentication.
#[utoipa::path(
    get,
    path = "/metrics",
    tag = "health",
    responses(
        (status = 200, description = "Prometheus metrics"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin access required"),
        (status = 503, description = "Metrics not available"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn prometheus_metrics<A>(
    _admin: AdminAuth,
    State(state): State<PgAppState<A>>,
) -> impl IntoResponse
where
    A: SessionService + 'static,
{
    // Update user and store count gauges before rendering
    if let Ok(count) = state.data_service.count_users().await {
        metrics::set_registered_users(count as u64);
    }
    if let Ok(count) = state.data_service.count_stores().await {
        metrics::set_stores(count as u64);
    }

    // Scrape DB pool stats
    let pool = state.data_service.pool();
    let pool_size = pool.size() as u64;
    let pool_idle = pool.num_idle() as u64;
    metrics::set_db_pool_connections("idle", pool_idle);
    metrics::set_db_pool_connections("used", pool_size.saturating_sub(pool_idle));

    match metrics::render() {
        Some(body) => (
            StatusCode::OK,
            [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
            body,
        ),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            [("content-type", "text/plain")],
            "Metrics not initialized".to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::too_many_lines,
        clippy::cognitive_complexity
    )]
    use super::*;

    // -- Response type serialization tests --

    #[test]
    fn readiness_ready_omits_failing() {
        let resp = ReadinessResponse {
            status: "ready".to_string(),
            failing: vec![],
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["status"], "ready");
        assert!(
            json.get("failing").is_none(),
            "empty failing should be omitted"
        );
    }

    #[test]
    fn readiness_not_ready_includes_failing() {
        let resp = ReadinessResponse {
            status: "not_ready".to_string(),
            failing: vec!["postgres".to_string(), "rpc:56".to_string()],
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["status"], "not_ready");
        let failing = json["failing"].as_array().unwrap();
        assert_eq!(failing.len(), 2);
        assert_eq!(failing[0], "postgres");
        assert_eq!(failing[1], "rpc:56");
    }

    #[test]
    fn deep_health_response_serialization() {
        let mut rpcs = HashMap::new();
        rpcs.insert(
            "1".to_string(),
            RpcHealth {
                status: "ok".to_string(),
                latency_ms: 12,
                last_block: Some(20_000_000),
                error: None,
            },
        );
        rpcs.insert(
            "56".to_string(),
            RpcHealth {
                status: "error".to_string(),
                latency_ms: 1000,
                last_block: None,
                error: Some("disconnected".to_string()),
            },
        );

        let resp = DeepHealthResponse {
            postgres: DependencyHealth {
                status: "ok".to_string(),
                latency_ms: 3,
                error: None,
            },
            redis: DependencyHealth {
                status: "ok".to_string(),
                latency_ms: 1,
                error: None,
            },
            rpcs,
            monitor: MonitorHealth {
                status: "ok".to_string(),
                data_fresh: true,
            },
        };

        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["postgres"]["status"], "ok");
        assert_eq!(json["postgres"]["latency_ms"], 3);
        assert!(json["postgres"].get("error").is_none());
        assert_eq!(json["redis"]["status"], "ok");
        assert_eq!(json["rpcs"]["1"]["status"], "ok");
        assert_eq!(json["rpcs"]["1"]["last_block"], 20_000_000);
        assert_eq!(json["rpcs"]["56"]["status"], "error");
        assert_eq!(json["rpcs"]["56"]["error"], "disconnected");
        assert!(json["rpcs"]["56"].get("last_block").is_none());
        assert_eq!(json["monitor"]["status"], "ok");
        assert_eq!(json["monitor"]["data_fresh"], true);
    }

    #[test]
    fn deep_health_no_monitor_configured() {
        let resp = DeepHealthResponse {
            postgres: DependencyHealth {
                status: "ok".to_string(),
                latency_ms: 2,
                error: None,
            },
            redis: DependencyHealth {
                status: "ok".to_string(),
                latency_ms: 0,
                error: Some("not configured".to_string()),
            },
            rpcs: HashMap::new(),
            monitor: MonitorHealth {
                status: "ok".to_string(),
                data_fresh: false,
            },
        };

        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["redis"]["error"], "not configured");
        assert!(json["rpcs"].as_object().unwrap().is_empty());
        assert_eq!(json["monitor"]["data_fresh"], false);
    }

    // -- ChainHealthInfo conversion tests --

    #[test]
    fn chain_health_connected_conversion() {
        let health = ChainHealth {
            chain_id: 1,
            chain_name: "Ethereum".to_string(),
            status: SourceStatus::Connected,
            current_block: Some(20_000_000),
            last_processed_block: Some(19_999_999),
            watched_addresses: 42,
            is_healthy: true,
        };
        let info: ChainHealthInfo = health.into();
        assert_eq!(info.status, "connected");
        assert!(info.is_healthy);
        assert_eq!(info.watched_addresses, 42);
    }

    #[test]
    fn chain_health_failed_conversion() {
        let health = ChainHealth {
            chain_id: 56,
            chain_name: "BSC".to_string(),
            status: SourceStatus::Failed("rpc timeout".to_string()),
            current_block: None,
            last_processed_block: None,
            watched_addresses: 0,
            is_healthy: false,
        };
        let info: ChainHealthInfo = health.into();
        assert_eq!(info.status, "failed: rpc timeout");
        assert!(!info.is_healthy);
    }

    #[test]
    fn chain_health_disconnected_conversion() {
        let health = ChainHealth {
            chain_id: 137,
            chain_name: "Polygon".to_string(),
            status: SourceStatus::Disconnected,
            current_block: Some(50_000_000),
            last_processed_block: Some(49_999_000),
            watched_addresses: 10,
            is_healthy: false,
        };
        let info: ChainHealthInfo = health.into();
        assert_eq!(info.status, "disconnected");
        assert!(!info.is_healthy);
    }

    #[test]
    fn readiness_response_roundtrip() {
        let resp = ReadinessResponse {
            status: "not_ready".to_string(),
            failing: vec!["redis".to_string()],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: ReadinessResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.status, "not_ready");
        assert_eq!(deserialized.failing, vec!["redis"]);
    }

    #[test]
    fn dependency_health_error_field_omitted_when_none() {
        let dep = DependencyHealth {
            status: "ok".to_string(),
            latency_ms: 5,
            error: None,
        };
        let json = serde_json::to_value(&dep).unwrap();
        assert!(json.get("error").is_none());
    }

    #[test]
    fn rpc_health_last_block_omitted_when_none() {
        let rpc = RpcHealth {
            status: "error".to_string(),
            latency_ms: 1000,
            last_block: None,
            error: Some("timeout".to_string()),
        };
        let json = serde_json::to_value(&rpc).unwrap();
        assert!(json.get("last_block").is_none());
        assert_eq!(json["error"], "timeout");
    }
}
