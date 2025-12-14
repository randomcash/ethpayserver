//! Device, passkey, wallet, and session management handlers.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::{
    AuthRepository, DeviceId, DeviceInfo, PasskeyId, PasskeyInfo, SessionId, WalletCredentialId,
    WalletInfo,
};

use super::AuthState;

#[derive(Debug, Deserialize, ToSchema)]
pub struct AuthenticatedRequest {
    pub session_id: SessionId,
}

#[utoipa::path(
    get,
    path = "/auth/devices",
    tag = "management",
    request_body = AuthenticatedRequest,
    responses(
        (status = 200, body = Vec<DeviceInfo>),
        (status = 401, description = "Invalid session"),
    )
)]
pub async fn list_devices<R: AuthRepository>(
    State(state): State<AuthState<R>>,
    Json(req): Json<AuthenticatedRequest>,
) -> Result<Json<Vec<DeviceInfo>>, (StatusCode, String)> {
    state
        .service
        .get_devices(req.session_id)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))
}

#[utoipa::path(
    delete,
    path = "/auth/devices/{id}",
    tag = "management",
    params(("id" = DeviceId, Path)),
    request_body = AuthenticatedRequest,
    responses(
        (status = 200, description = "Device revoked"),
        (status = 400, description = "Cannot revoke current device"),
    )
)]
pub async fn revoke_device<R: AuthRepository>(
    State(state): State<AuthState<R>>,
    Path(device_id): Path<DeviceId>,
    Json(req): Json<AuthenticatedRequest>,
) -> Result<Json<()>, (StatusCode, String)> {
    state
        .service
        .revoke_device(req.session_id, device_id)
        .await
        .map(|_| Json(()))
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[utoipa::path(
    get,
    path = "/auth/passkeys",
    tag = "management",
    request_body = AuthenticatedRequest,
    responses(
        (status = 200, body = Vec<PasskeyInfo>),
        (status = 401, description = "Invalid session"),
    )
)]
pub async fn list_passkeys<R: AuthRepository>(
    State(state): State<AuthState<R>>,
    Json(req): Json<AuthenticatedRequest>,
) -> Result<Json<Vec<PasskeyInfo>>, (StatusCode, String)> {
    state
        .service
        .get_passkeys(req.session_id)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))
}

#[utoipa::path(
    delete,
    path = "/auth/passkeys/{id}",
    tag = "management",
    params(("id" = PasskeyId, Path)),
    request_body = AuthenticatedRequest,
    responses(
        (status = 200, description = "Passkey revoked"),
        (status = 400, description = "Cannot revoke last passkey"),
    )
)]
pub async fn revoke_passkey<R: AuthRepository>(
    State(state): State<AuthState<R>>,
    Path(passkey_id): Path<PasskeyId>,
    Json(req): Json<AuthenticatedRequest>,
) -> Result<Json<()>, (StatusCode, String)> {
    state
        .service
        .revoke_passkey(req.session_id, passkey_id)
        .await
        .map(|_| Json(()))
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[utoipa::path(
    get,
    path = "/auth/wallets",
    tag = "management",
    request_body = AuthenticatedRequest,
    responses(
        (status = 200, body = Vec<WalletInfo>),
        (status = 401, description = "Invalid session"),
    )
)]
pub async fn list_wallets<R: AuthRepository>(
    State(state): State<AuthState<R>>,
    Json(req): Json<AuthenticatedRequest>,
) -> Result<Json<Vec<WalletInfo>>, (StatusCode, String)> {
    state
        .service
        .get_wallets(req.session_id)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))
}

#[utoipa::path(
    delete,
    path = "/auth/wallets/{id}",
    tag = "management",
    params(("id" = WalletCredentialId, Path)),
    request_body = AuthenticatedRequest,
    responses(
        (status = 200, description = "Wallet revoked"),
        (status = 400, description = "Cannot revoke last wallet"),
    )
)]
pub async fn revoke_wallet<R: AuthRepository>(
    State(state): State<AuthState<R>>,
    Path(wallet_id): Path<WalletCredentialId>,
    Json(req): Json<AuthenticatedRequest>,
) -> Result<Json<()>, (StatusCode, String)> {
    state
        .service
        .revoke_wallet(req.session_id, wallet_id)
        .await
        .map(|_| Json(()))
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[utoipa::path(
    post,
    path = "/auth/logout",
    tag = "management",
    request_body = AuthenticatedRequest,
    responses((status = 200, description = "Logged out"))
)]
pub async fn logout<R: AuthRepository>(
    State(state): State<AuthState<R>>,
    Json(req): Json<AuthenticatedRequest>,
) -> Result<Json<()>, (StatusCode, String)> {
    state
        .service
        .logout(req.session_id)
        .await
        .map(|_| Json(()))
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[utoipa::path(
    post,
    path = "/auth/logout/all",
    tag = "management",
    request_body = AuthenticatedRequest,
    responses((status = 200, description = "Logged out from all sessions"))
)]
pub async fn logout_all<R: AuthRepository>(
    State(state): State<AuthState<R>>,
    Json(req): Json<AuthenticatedRequest>,
) -> Result<Json<()>, (StatusCode, String)> {
    state
        .service
        .logout_all(req.session_id)
        .await
        .map(|_| Json(()))
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}
