//! Network information endpoints.
//!
//! These endpoints are public and do not require authentication.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use auth::AuthRepository;
use serde::Serialize;
use utoipa::ToSchema;

use super::{EvmDataService, EvmState};
use crate::network::{ChainConfig, ALL_CHAINS};
use crate::EvmNetwork;

/// Network information.
#[derive(Debug, Serialize, ToSchema)]
pub struct NetworkInfo {
    /// Network identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Chain ID.
    pub chain_id: u64,
    /// Native token symbol.
    pub native_symbol: String,
    /// Block time in seconds.
    pub block_time_secs: u64,
    /// Required confirmations.
    pub confirmations: u32,
    /// Block explorer URL.
    pub explorer_url: String,
}

impl From<&ChainConfig> for NetworkInfo {
    fn from(c: &ChainConfig) -> Self {
        Self {
            id: c.network.as_str().to_string(),
            name: c.name.to_string(),
            chain_id: c.chain_id,
            native_symbol: c.native_symbol.to_string(),
            block_time_secs: c.block_time_secs,
            confirmations: c.confirmations_required,
            explorer_url: c.explorer_url.to_string(),
        }
    }
}

/// Network list response.
#[derive(Debug, Serialize, ToSchema)]
pub struct NetworkListResponse {
    pub networks: Vec<NetworkInfo>,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, String)>;

#[utoipa::path(
    get,
    path = "/evm/networks",
    tag = "networks",
    responses(
        (status = 200, body = NetworkListResponse),
    )
)]
pub async fn list_networks<D, R>(
    State(_state): State<EvmState<D, R>>,
) -> ApiResult<NetworkListResponse>
where
    D: EvmDataService + 'static,
    R: AuthRepository + Send + Sync + 'static,
{
    let networks = ALL_CHAINS.iter().map(NetworkInfo::from).collect();
    Ok(Json(NetworkListResponse { networks }))
}

#[utoipa::path(
    get,
    path = "/evm/networks/{network}",
    tag = "networks",
    params(("network" = String, Path, description = "Network identifier (e.g., ethereum, polygon)")),
    responses(
        (status = 200, body = NetworkInfo),
        (status = 404, description = "Network not found"),
    )
)]
pub async fn get_network<D, R>(
    State(_state): State<EvmState<D, R>>,
    Path(network): Path<String>,
) -> ApiResult<NetworkInfo>
where
    D: EvmDataService + 'static,
    R: AuthRepository + Send + Sync + 'static,
{
    let evm_network: EvmNetwork = network
        .parse()
        .map_err(|_| (StatusCode::NOT_FOUND, format!("network '{}' not found", network)))?;

    let config = crate::get_chain_config(evm_network);
    Ok(Json(NetworkInfo::from(config)))
}
