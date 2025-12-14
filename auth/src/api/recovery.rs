//! Account recovery handlers.

use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::{
    AuthRepository, CompleteRecoveryRequest, LoginResponse, StartPasskeyRegistrationResponse,
    StartRecoveryRequest,
};

use super::AuthState;

#[utoipa::path(
    post,
    path = "/auth/recovery/start",
    tag = "recovery",
    request_body = StartRecoveryRequest,
    responses(
        (status = 200, description = "Recovery challenge created", body = StartPasskeyRegistrationResponse),
        (status = 400, description = "Invalid identifier or recovery hash"),
    )
)]
pub async fn start_recovery<R: AuthRepository>(
    State(state): State<AuthState<R>>,
    Json(req): Json<StartRecoveryRequest>,
) -> Result<Json<StartPasskeyRegistrationResponse>, (StatusCode, String)> {
    state
        .service
        .start_account_recovery(req)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CompleteRecoveryRequestBody {
    #[schema(example = "user@example.com")]
    pub identifier: String,
    #[serde(flatten)]
    pub request: CompleteRecoveryRequest,
}

#[utoipa::path(
    post,
    path = "/auth/recovery/complete",
    tag = "recovery",
    request_body = CompleteRecoveryRequestBody,
    responses(
        (status = 200, description = "Recovery complete", body = LoginResponse),
        (status = 400, description = "Invalid credential"),
    )
)]
pub async fn complete_recovery<R: AuthRepository>(
    State(state): State<AuthState<R>>,
    Json(req): Json<CompleteRecoveryRequestBody>,
) -> Result<Json<LoginResponse>, (StatusCode, String)> {
    state
        .service
        .complete_account_recovery(&req.identifier, req.request)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}
