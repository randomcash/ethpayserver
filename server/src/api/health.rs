//! Health check endpoints.

use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

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
