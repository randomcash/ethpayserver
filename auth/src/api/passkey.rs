//! Passkey (WebAuthn) authentication handlers.

use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::{
    AuthRepository, CompleteNewUserPasskeyRegistrationRequest, CompletePasskeyLoginRequest,
    CompletePasskeyRegistrationRequest, LoginResponse, PasskeyInfo, SessionId,
    StartNewUserPasskeyRegistrationResponse, StartPasskeyLoginResponse,
    StartPasskeyRegistrationRequest, StartPasskeyRegistrationResponse,
};

use super::AuthState;

#[derive(Debug, Deserialize, ToSchema)]
pub struct StartNewUserRequest {
    #[schema(example = "user@example.com")]
    pub email: String,
}

#[utoipa::path(
    post,
    path = "/auth/passkey/new-user/start",
    tag = "passkey",
    request_body = StartNewUserRequest,
    responses(
        (status = 200, description = "Challenge created", body = StartNewUserPasskeyRegistrationResponse),
        (status = 400, description = "Invalid email or user exists"),
    )
)]
pub async fn start_new_user_registration<R: AuthRepository>(
    State(state): State<AuthState<R>>,
    Json(req): Json<StartNewUserRequest>,
) -> Result<Json<StartNewUserPasskeyRegistrationResponse>, (StatusCode, String)> {
    state
        .service
        .start_new_user_passkey_registration(&req.email)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[utoipa::path(
    post,
    path = "/auth/passkey/new-user/complete",
    tag = "passkey",
    request_body = CompleteNewUserPasskeyRegistrationRequest,
    responses(
        (status = 200, description = "Registration complete", body = LoginResponse),
        (status = 400, description = "Invalid credential"),
    )
)]
pub async fn complete_new_user_registration<R: AuthRepository>(
    State(state): State<AuthState<R>>,
    Json(req): Json<CompleteNewUserPasskeyRegistrationRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, String)> {
    state
        .service
        .complete_new_user_passkey_registration(req)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct StartRegistrationRequest {
    pub session_id: SessionId,
    #[serde(flatten)]
    pub request: StartPasskeyRegistrationRequest,
}

#[utoipa::path(
    post,
    path = "/auth/passkey/register/start",
    tag = "passkey",
    request_body = StartRegistrationRequest,
    responses(
        (status = 200, description = "Challenge created", body = StartPasskeyRegistrationResponse),
        (status = 400, description = "Invalid session"),
    )
)]
pub async fn start_registration<R: AuthRepository>(
    State(state): State<AuthState<R>>,
    Json(req): Json<StartRegistrationRequest>,
) -> Result<Json<StartPasskeyRegistrationResponse>, (StatusCode, String)> {
    state
        .service
        .start_passkey_registration(req.session_id, req.request)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CompleteRegistrationRequest {
    pub session_id: SessionId,
    #[serde(flatten)]
    pub request: CompletePasskeyRegistrationRequest,
}

#[utoipa::path(
    post,
    path = "/auth/passkey/register/complete",
    tag = "passkey",
    request_body = CompleteRegistrationRequest,
    responses(
        (status = 200, description = "Passkey registered", body = PasskeyInfo),
        (status = 400, description = "Invalid credential"),
    )
)]
pub async fn complete_registration<R: AuthRepository>(
    State(state): State<AuthState<R>>,
    Json(req): Json<CompleteRegistrationRequest>,
) -> Result<Json<PasskeyInfo>, (StatusCode, String)> {
    state
        .service
        .complete_passkey_registration(req.session_id, req.request)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct StartLoginRequest {
    #[schema(example = "user@example.com")]
    pub email: String,
}

#[utoipa::path(
    post,
    path = "/auth/passkey/login/start",
    tag = "passkey",
    request_body = StartLoginRequest,
    responses(
        (status = 200, description = "Challenge created", body = StartPasskeyLoginResponse),
        (status = 400, description = "User not found"),
    )
)]
pub async fn start_login<R: AuthRepository>(
    State(state): State<AuthState<R>>,
    Json(req): Json<StartLoginRequest>,
) -> Result<Json<StartPasskeyLoginResponse>, (StatusCode, String)> {
    state
        .service
        .start_passkey_login(&req.email)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[utoipa::path(
    post,
    path = "/auth/passkey/login/complete",
    tag = "passkey",
    request_body = CompletePasskeyLoginRequest,
    responses(
        (status = 200, description = "Login successful", body = LoginResponse),
        (status = 400, description = "Invalid credential"),
    )
)]
pub async fn complete_login<R: AuthRepository>(
    State(state): State<AuthState<R>>,
    Json(req): Json<CompletePasskeyLoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, String)> {
    state
        .service
        .complete_passkey_login(req)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}
