//! HTTP API for EVM operations.
//!
//! # Usage
//!
//! ```rust,ignore
//! use evm::api::{EvmState, router};
//! use data_service::PgDataService;
//! use auth::{AuthService, AuthRepository};
//! use std::sync::Arc;
//!
//! let data_service = Arc::new(PgDataService::connect("...").await?);
//! let auth_service = Arc::new(AuthService::new(auth_repo));
//! let state = EvmState::new(data_service, auth_service);
//! let app = Router::new().nest("/evm", router(state));
//! ```

use std::sync::Arc;

use axum::{
    routing::{delete, get, post, put},
    Router,
};
use data_service::PgDataService;
use auth::{AuthRepository, AuthService};
use utoipa::OpenApi;

mod extractors;
pub mod networks;
pub mod tokens;

pub use extractors::AdminAuth;

/// Shared state for EVM API handlers.
///
/// Generic over the auth repository type `R`.
pub struct EvmState<R: AuthRepository> {
    pub data_service: Arc<PgDataService>,
    pub auth_service: Arc<AuthService<R>>,
}

impl<R: AuthRepository> Clone for EvmState<R> {
    fn clone(&self) -> Self {
        Self {
            data_service: Arc::clone(&self.data_service),
            auth_service: Arc::clone(&self.auth_service),
        }
    }
}

impl<R: AuthRepository> EvmState<R> {
    pub fn new(data_service: Arc<PgDataService>, auth_service: Arc<AuthService<R>>) -> Self {
        Self { data_service, auth_service }
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
///
/// All token management endpoints require admin authentication.
/// Network info endpoints are public (no auth required).
pub fn router<R>(state: EvmState<R>) -> Router
where
    R: AuthRepository + Send + Sync + 'static,
{
    Router::new()
        // Token endpoints (admin only)
        .route("/tokens", get(tokens::list_tokens::<R>))
        .route("/tokens", post(tokens::create_token::<R>))
        .route("/tokens/{id}", get(tokens::get_token::<R>))
        .route("/tokens/{id}", put(tokens::update_token::<R>))
        .route("/tokens/{id}", delete(tokens::delete_token::<R>))
        .route("/tokens/{id}/enabled", put(tokens::set_token_enabled::<R>))
        // Network endpoints (public)
        .route("/networks", get(networks::list_networks::<R>))
        .route("/networks/{network}", get(networks::get_network::<R>))
        .with_state(state)
}
