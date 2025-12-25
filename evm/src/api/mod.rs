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
use auth::SessionService;
use types::{TokenReader, TokenWriter};
use utoipa::OpenApi;

mod extractors;
pub mod networks;
pub mod tokens;

pub use extractors::AdminAuth;

/// Read-only data service trait for EVM API.
///
/// Use this bound for handlers that only read from the database.
pub trait EvmDataServiceReader: TokenReader + Send + Sync {}

impl<T> EvmDataServiceReader for T where T: TokenReader + Send + Sync {}

/// Full data service trait for EVM API (read + write).
///
/// Use this bound for handlers that modify the database.
pub trait EvmDataService: EvmDataServiceReader + TokenWriter {}

impl<T> EvmDataService for T where T: EvmDataServiceReader + TokenWriter {}

/// Shared state for EVM API handlers.
///
/// Generic over the data service type `D` and auth service type `A`.
/// `D` bounds are placed on handlers to allow read-only vs read-write separation.
/// `A` must implement `SessionService` for session validation.
pub struct EvmState<D, A> {
    pub data_service: Arc<D>,
    pub auth_service: Arc<A>,
}

impl<D, A> Clone for EvmState<D, A> {
    fn clone(&self) -> Self {
        Self {
            data_service: Arc::clone(&self.data_service),
            auth_service: Arc::clone(&self.auth_service),
        }
    }
}

impl<D, A> EvmState<D, A> {
    pub fn new(data_service: Arc<D>, auth_service: Arc<A>) -> Self {
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
pub fn router<D, A>(state: EvmState<D, A>) -> Router
where
    D: EvmDataService + 'static,
    A: SessionService + 'static,
{
    Router::new()
        // Token endpoints (admin only)
        .route("/tokens", get(tokens::list_tokens::<D, A>))
        .route("/tokens", post(tokens::create_token::<D, A>))
        .route("/tokens/{id}", get(tokens::get_token::<D, A>))
        .route("/tokens/{id}", put(tokens::update_token::<D, A>))
        .route("/tokens/{id}", delete(tokens::delete_token::<D, A>))
        .route("/tokens/{id}/enabled", put(tokens::set_token_enabled::<D, A>))
        // Network endpoints (public)
        .route("/networks", get(networks::list_networks::<D, A>))
        .route("/networks/{network}", get(networks::get_network::<D, A>))
        .with_state(state)
}
