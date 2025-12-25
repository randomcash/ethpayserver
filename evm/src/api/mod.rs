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
use auth::{AuthRepository, AuthService};
use types::{TokenReader, TokenWriter};
use utoipa::OpenApi;

mod extractors;
pub mod networks;
pub mod tokens;

pub use extractors::AdminAuth;

/// Trait for data service requirements in EVM API.
pub trait EvmDataService: TokenReader + TokenWriter + Send + Sync {}

impl<T> EvmDataService for T where T: TokenReader + TokenWriter + Send + Sync {}

/// Shared state for EVM API handlers.
///
/// Generic over the data service type `D` and auth repository type `R`.
pub struct EvmState<D: EvmDataService, R: AuthRepository> {
    pub data_service: Arc<D>,
    pub auth_service: Arc<AuthService<R>>,
}

impl<D: EvmDataService, R: AuthRepository> Clone for EvmState<D, R> {
    fn clone(&self) -> Self {
        Self {
            data_service: Arc::clone(&self.data_service),
            auth_service: Arc::clone(&self.auth_service),
        }
    }
}

impl<D: EvmDataService, R: AuthRepository> EvmState<D, R> {
    pub fn new(data_service: Arc<D>, auth_service: Arc<AuthService<R>>) -> Self {
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
pub fn router<D, R>(state: EvmState<D, R>) -> Router
where
    D: EvmDataService + 'static,
    R: AuthRepository + Send + Sync + 'static,
{
    Router::new()
        // Token endpoints (admin only)
        .route("/tokens", get(tokens::list_tokens::<D, R>))
        .route("/tokens", post(tokens::create_token::<D, R>))
        .route("/tokens/{id}", get(tokens::get_token::<D, R>))
        .route("/tokens/{id}", put(tokens::update_token::<D, R>))
        .route("/tokens/{id}", delete(tokens::delete_token::<D, R>))
        .route("/tokens/{id}/enabled", put(tokens::set_token_enabled::<D, R>))
        // Network endpoints (public)
        .route("/networks", get(networks::list_networks::<D, R>))
        .route("/networks/{network}", get(networks::get_network::<D, R>))
        .with_state(state)
}
