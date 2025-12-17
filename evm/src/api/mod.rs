//! HTTP API for EVM operations.
//!
//! # Usage
//!
//! ```rust,ignore
//! use evm::api::{EvmState, router};
//! use data_service::PgDataService;
//! use std::sync::Arc;
//!
//! let data_service = Arc::new(PgDataService::connect("...").await?);
//! let state = EvmState::new(data_service);
//! let app = Router::new().nest("/evm", router(state));
//! ```

use std::sync::Arc;

use axum::{
    routing::{delete, get, post, put},
    Router,
};
use data_service::PgDataService;
use utoipa::OpenApi;

pub mod networks;
pub mod tokens;

/// Shared state for EVM API handlers.
pub struct EvmState {
    pub data_service: Arc<PgDataService>,
}

impl Clone for EvmState {
    fn clone(&self) -> Self {
        Self {
            data_service: Arc::clone(&self.data_service),
        }
    }
}

impl EvmState {
    pub fn new(data_service: Arc<PgDataService>) -> Self {
        Self { data_service }
    }
}

/// OpenAPI documentation for EVM endpoints.
#[derive(OpenApi)]
#[openapi(
    info(title = "EVM API", version = "0.1.0", license(name = "MIT")),
    paths(
        // Tokens
        tokens::list_tokens,
        tokens::get_token,
        tokens::create_token,
        tokens::update_token,
        tokens::delete_token,
        tokens::set_token_enabled,
        // Networks
        networks::list_networks,
        networks::get_network,
    ),
    components(schemas(
        tokens::CreateTokenRequest,
        tokens::UpdateTokenRequest,
        tokens::SetEnabledRequest,
        tokens::TokenResponse,
        tokens::TokenListResponse,
        networks::NetworkInfo,
        networks::NetworkListResponse,
    )),
    tags(
        (name = "tokens", description = "Token management"),
        (name = "networks", description = "Network information"),
    )
)]
pub struct EvmApiDoc;

/// Create the EVM router. Mount at `/evm`.
pub fn router(state: EvmState) -> Router {
    Router::new()
        // Token endpoints
        .route("/tokens", get(tokens::list_tokens))
        .route("/tokens", post(tokens::create_token))
        .route("/tokens/{id}", get(tokens::get_token))
        .route("/tokens/{id}", put(tokens::update_token))
        .route("/tokens/{id}", delete(tokens::delete_token))
        .route("/tokens/{id}/enabled", put(tokens::set_token_enabled))
        // Network endpoints
        .route("/networks", get(networks::list_networks))
        .route("/networks/{network}", get(networks::get_network))
        .with_state(state)
}
