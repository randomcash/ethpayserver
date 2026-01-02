//! Health check endpoints.

use auth::SessionService;
use axum::{extract::State, http::StatusCode, Json, response::IntoResponse};
use evm::monitor::{ChainHealth, SourceStatus};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::AdminAuth;
use crate::metrics;
use crate::services::EVMMonitor;
use crate::state::PgAppState;

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
pub async fn health_check<A>(State(state): State<PgAppState<A>>) -> (StatusCode, Json<HealthResponse>)
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
        status: if all_healthy { "healthy".to_string() } else { "unhealthy".to_string() },
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

/// Readiness probe.
///
/// Returns 200 if the service is ready to accept traffic.
#[utoipa::path(
    get,
    path = "/health/ready",
    tag = "health",
    responses(
        (status = 200, description = "Service is ready"),
        (status = 503, description = "Service is not ready"),
    )
)]
pub async fn readiness<A>(State(state): State<PgAppState<A>>) -> StatusCode
where
    A: Send + Sync + 'static,
{
    let db_ok = state.data_service.health_check().await.is_ok();

    // Check Redis if configured (optional for readiness)
    let redis_ok = if let Some(ref monitor) = state.evm_monitor {
        monitor.health_check().await.is_ok()
    } else {
        true // No EVM monitor configured, so it's OK
    };

    if db_ok && redis_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
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
