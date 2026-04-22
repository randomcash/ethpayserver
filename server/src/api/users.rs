//! User API endpoints — API key management.
//!
//! All endpoints require authentication via session token.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use auth::{ApiKey, ApiKeyId, ApiKeyInfo, ApiKeyRepository, Role, SessionService};

use super::extractors::AuthenticatedUser;
use crate::state::PgAppState;

/// Response for listing API keys.
#[derive(Debug, Serialize, ToSchema)]
pub struct ApiKeyListResponse {
    pub keys: Vec<ApiKeyInfoResponse>,
}

/// API key info for list/get responses.
#[derive(Debug, Serialize, ToSchema)]
pub struct ApiKeyInfoResponse {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    /// Per-key rate limit in requests per minute. Null = server default.
    pub rate_limit_rpm: Option<i32>,
}

impl ApiKeyInfoResponse {
    fn from_key_with_rate_limit(key: &ApiKey, rate_limit_rpm: Option<i32>) -> Self {
        let info = ApiKeyInfo::from(key);
        Self {
            id: info.id.0,
            name: info.name,
            key_prefix: info.key_prefix,
            is_active: info.is_active,
            created_at: info.created_at,
            last_used_at: info.last_used_at,
            expires_at: info.expires_at,
            rate_limit_rpm,
        }
    }
}

impl From<ApiKeyInfo> for ApiKeyInfoResponse {
    fn from(info: ApiKeyInfo) -> Self {
        Self {
            id: info.id.0,
            name: info.name,
            key_prefix: info.key_prefix,
            is_active: info.is_active,
            created_at: info.created_at,
            last_used_at: info.last_used_at,
            expires_at: info.expires_at,
            rate_limit_rpm: None,
        }
    }
}

/// Request to create a new API key.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateApiKeyPayload {
    /// Human-readable name for the key.
    pub name: String,
    /// Optional expiration time.
    pub expires_at: Option<DateTime<Utc>>,
}

/// Response after creating an API key (includes plaintext key).
#[derive(Debug, Serialize, ToSchema)]
pub struct CreateApiKeyResponsePayload {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    /// The plaintext API key. Store this securely — it cannot be retrieved again.
    pub key: String,
}

/// List all API keys for the authenticated user.
#[utoipa::path(
    get,
    path = "/users/api-keys",
    tag = "users",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "API keys listed", body = ApiKeyListResponse),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn list_api_keys<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
) -> Result<Json<ApiKeyListResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let keys = state
        .data_service
        .list_user_api_keys_with_rate_limit(user.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let keys = keys
        .iter()
        .map(|(k, rpm)| ApiKeyInfoResponse::from_key_with_rate_limit(k, *rpm))
        .collect();

    Ok(Json(ApiKeyListResponse { keys }))
}

/// Create a new API key for the authenticated user.
#[utoipa::path(
    post,
    path = "/users/api-keys",
    tag = "users",
    security(("bearer_auth" = [])),
    request_body = CreateApiKeyPayload,
    responses(
        (status = 201, description = "API key created", body = CreateApiKeyResponsePayload),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn create_api_key<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Json(payload): Json<CreateApiKeyPayload>,
) -> Result<(StatusCode, Json<CreateApiKeyResponsePayload>), StatusCode>
where
    A: SessionService + 'static,
{
    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Generate a random API key
    let raw_key = format!(
        "ak_{}_{}",
        generate_key_segment(4),
        generate_key_segment(32)
    );
    let key_prefix = format!("{}****{}", &raw_key[..8], &raw_key[raw_key.len() - 4..]);

    // Hash the key for storage
    let key_hash = hash_api_key(&raw_key);

    let now = Utc::now();
    let api_key = ApiKey {
        id: ApiKeyId::new(),
        user_id: user.id,
        name: name.clone(),
        key_hash,
        key_prefix: key_prefix.clone(),
        is_active: true,
        created_at: now,
        last_used_at: None,
        expires_at: payload.expires_at,
    };

    state
        .data_service
        .create_api_key(&api_key)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok((
        StatusCode::CREATED,
        Json(CreateApiKeyResponsePayload {
            id: api_key.id.0,
            name,
            key_prefix,
            is_active: true,
            created_at: now,
            expires_at: payload.expires_at,
            key: raw_key,
        }),
    ))
}

/// Revoke (deactivate) an API key.
#[utoipa::path(
    delete,
    path = "/users/api-keys/{id}",
    tag = "users",
    security(("bearer_auth" = [])),
    params(
        ("id" = Uuid, Path, description = "API key ID to revoke"),
    ),
    responses(
        (status = 204, description = "API key revoked"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "API key not found"),
    )
)]
pub async fn revoke_api_key<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(id): Path<Uuid>,
) -> StatusCode
where
    A: SessionService + 'static,
{
    // Verify the key belongs to this user
    let key = state
        .data_service
        .get_api_key(ApiKeyId(id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);

    let key = match key {
        Ok(Some(k)) => k,
        Ok(None) => return StatusCode::NOT_FOUND,
        Err(status) => return status,
    };

    if key.user_id != user.id {
        return StatusCode::NOT_FOUND;
    }

    match state.data_service.revoke_api_key(ApiKeyId(id)).await {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Request to update an API key's rate limit.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateApiKeyPayload {
    /// Per-key rate limit in requests per minute. Null = use server default.
    pub rate_limit_rpm: Option<i32>,
}

/// Update an API key's settings (currently: rate_limit_rpm).
#[utoipa::path(
    patch,
    path = "/users/api-keys/{id}",
    tag = "users",
    security(("bearer_auth" = [])),
    params(
        ("id" = Uuid, Path, description = "API key ID to update"),
    ),
    request_body = UpdateApiKeyPayload,
    responses(
        (status = 200, description = "API key updated", body = ApiKeyInfoResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "API key not found"),
    )
)]
pub async fn update_api_key<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateApiKeyPayload>,
) -> Result<Json<ApiKeyInfoResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    // Validate rate_limit_rpm if set
    if let Some(rpm) = payload.rate_limit_rpm
        && rpm < 1
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Verify the key belongs to this user
    let key = state
        .data_service
        .get_api_key(ApiKeyId(id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let key = match key {
        Some(k) => k,
        None => return Err(StatusCode::NOT_FOUND),
    };

    if key.user_id != user.id && user.role != Role::ServerAdmin {
        return Err(StatusCode::NOT_FOUND);
    }

    state
        .data_service
        .update_api_key_rate_limit(ApiKeyId(id), payload.rate_limit_rpm)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(ApiKeyInfoResponse::from_key_with_rate_limit(
        &key,
        payload.rate_limit_rpm,
    )))
}

/// Generate a random hex segment for API key generation.
fn generate_key_segment(bytes: usize) -> String {
    use std::fmt::Write;
    let mut buf = vec![0u8; bytes];
    // getrandom only fails if the system has no RNG. If that's happened we
    // have much bigger problems than this function — surface it and crash
    // cleanly rather than silently minting a zero-entropy key.
    if getrandom::fill(&mut buf).is_err() {
        // Zeroed buffer would be a catastrophic key; panic here rather than
        // return weak entropy. This is an init-time invariant.
        #[allow(clippy::panic, reason = "system RNG missing is unrecoverable")]
        {
            panic!("getrandom failed — refusing to mint weak API key");
        }
    }
    let mut s = String::with_capacity(bytes * 2);
    for b in &buf {
        // write! on String can only fail on OOM — fmt::Write for String is
        // infallible in practice.
        #[allow(clippy::unwrap_used, reason = "writing to String is infallible")]
        write!(s, "{:02x}", b).unwrap();
    }
    s
}

/// Hash an API key with SHA-256 for storage.
fn hash_api_key(raw_key: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(raw_key.as_bytes());
    hex::encode(hasher.finalize())
}
