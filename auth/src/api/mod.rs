//! HTTP API for authentication endpoints.
//!
//! # Usage
//!
//! ```rust,ignore
//! use auth::{api, AuthService};
//!
//! let service = Arc::new(AuthService::new(repo));
//! let state = api::AuthState::new(service);
//! let app = Router::new().nest("/auth", api::router(state));
//!
//! // OpenAPI spec
//! let openapi = auth::AuthApiDoc::openapi();
//! ```

use std::sync::Arc;

use axum::{
    routing::{delete, get, post},
    Router,
};
use utoipa::OpenApi;

use crate::{AuthRepository, AuthService};

pub mod management;
pub mod passkey;
pub mod recovery;
pub mod wallet;

/// Shared state for auth handlers.
pub struct AuthState<R: AuthRepository> {
    pub service: Arc<AuthService<R>>,
}

impl<R: AuthRepository> Clone for AuthState<R> {
    fn clone(&self) -> Self {
        Self { service: Arc::clone(&self.service) }
    }
}

impl<R: AuthRepository> AuthState<R> {
    pub fn new(service: Arc<AuthService<R>>) -> Self {
        Self { service }
    }
}

/// OpenAPI documentation for auth endpoints.
#[derive(OpenApi)]
#[openapi(
    info(title = "Auth API", version = "0.1.0", license(name = "MIT")),
    paths(
        passkey::start_new_user_registration,
        passkey::complete_new_user_registration,
        passkey::start_registration,
        passkey::complete_registration,
        passkey::start_login,
        passkey::complete_login,
        wallet::start_new_user_registration,
        wallet::complete_new_user_registration,
        wallet::start_registration,
        wallet::complete_registration,
        wallet::start_login,
        wallet::complete_login,
        recovery::start_recovery,
        recovery::complete_recovery,
        management::list_devices,
        management::revoke_device,
        management::list_passkeys,
        management::revoke_passkey,
        management::list_wallets,
        management::revoke_wallet,
        management::logout,
        management::logout_all,
    ),
    components(schemas(
        passkey::StartNewUserRequest,
        passkey::StartRegistrationRequest,
        passkey::CompleteRegistrationRequest,
        passkey::StartLoginRequest,
        wallet::StartRegistrationRequest,
        wallet::CompleteRegistrationRequest,
        recovery::CompleteRecoveryRequestBody,
        management::AuthenticatedRequest,
        crate::UserId,
        crate::SessionId,
        crate::DeviceId,
        crate::DeviceInfo,
        crate::DeviceType,
        crate::PasskeyId,
        crate::PasskeyInfo,
        crate::WalletCredentialId,
        crate::WalletInfo,
        crate::LoginResponse,
    )),
    tags(
        (name = "passkey", description = "WebAuthn authentication"),
        (name = "wallet", description = "Ethereum wallet authentication"),
        (name = "recovery", description = "Account recovery"),
        (name = "management", description = "Device and credential management"),
    )
)]
pub struct AuthApiDoc;

/// Create the auth router. Mount at `/auth`.
pub fn router<R: AuthRepository + 'static>(state: AuthState<R>) -> Router {
    Router::new()
        .route("/passkey/new-user/start", post(passkey::start_new_user_registration))
        .route("/passkey/new-user/complete", post(passkey::complete_new_user_registration))
        .route("/passkey/register/start", post(passkey::start_registration))
        .route("/passkey/register/complete", post(passkey::complete_registration))
        .route("/passkey/login/start", post(passkey::start_login))
        .route("/passkey/login/complete", post(passkey::complete_login))
        .route("/wallet/new-user/start", post(wallet::start_new_user_registration))
        .route("/wallet/new-user/complete", post(wallet::complete_new_user_registration))
        .route("/wallet/register/start", post(wallet::start_registration))
        .route("/wallet/register/complete", post(wallet::complete_registration))
        .route("/wallet/login/start", post(wallet::start_login))
        .route("/wallet/login/complete", post(wallet::complete_login))
        .route("/recovery/start", post(recovery::start_recovery))
        .route("/recovery/complete", post(recovery::complete_recovery))
        .route("/devices", get(management::list_devices))
        .route("/devices/{id}", delete(management::revoke_device))
        .route("/passkeys", get(management::list_passkeys))
        .route("/passkeys/{id}", delete(management::revoke_passkey))
        .route("/wallets", get(management::list_wallets))
        .route("/wallets/{id}", delete(management::revoke_wallet))
        .route("/logout", post(management::logout))
        .route("/logout/all", post(management::logout_all))
        .with_state(state)
}
