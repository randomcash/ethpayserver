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
use data_service::ApiKeyFullInfo;

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
    /// Set when the key is deprecated via rotation. Key remains valid during
    /// the grace window; null means not deprecated.
    pub deprecated_at: Option<DateTime<Utc>>,
}

impl ApiKeyInfoResponse {
    /// Build from an `ApiKey` plus the ancillary rate-limit / deprecation fields
    /// not present on the auth-crate struct. Used by endpoints that already
    /// have an `ApiKey` in hand (e.g. update_api_key after a mutation).
    fn from_key_with_rate_limit(
        key: &ApiKey,
        rate_limit_rpm: Option<i32>,
        deprecated_at: Option<DateTime<Utc>>,
    ) -> Self {
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
            deprecated_at,
        }
    }
}

impl From<ApiKeyFullInfo> for ApiKeyInfoResponse {
    fn from(info: ApiKeyFullInfo) -> Self {
        Self {
            id: info.id,
            name: info.name,
            key_prefix: info.key_prefix,
            is_active: info.is_active,
            created_at: info.created_at,
            last_used_at: info.last_used_at,
            expires_at: info.expires_at,
            rate_limit_rpm: info.rate_limit_rpm,
            deprecated_at: info.deprecated_at,
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

/// Response after rotating an API key.
#[derive(Debug, Serialize, ToSchema)]
pub struct RotateApiKeyResponsePayload {
    /// The new API key's ID.
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub created_at: DateTime<Utc>,
    /// The new plaintext API key. Store this securely.
    pub key: String,
    /// When the old key was deprecated (grace window starts here).
    pub old_key_deprecated_at: DateTime<Utc>,
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
        .list_user_api_keys_full(user.id.0)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let keys = keys.into_iter().map(ApiKeyInfoResponse::from).collect();

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

    let (raw_key, api_key) = build_api_key(&name, user.id, payload.expires_at);

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
            key_prefix: api_key.key_prefix,
            is_active: true,
            created_at: api_key.created_at,
            expires_at: api_key.expires_at,
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

    // Preserve any existing deprecation state in the response rather than
    // always returning None — prevents a stale-UI bug where the client
    // thinks the key was un-deprecated after a rate-limit update.
    let deprecated_at = state
        .data_service
        .get_api_key_auth_info_by_id(id)
        .await
        .ok()
        .flatten()
        .and_then(|info| info.deprecated_at);

    Ok(Json(ApiKeyInfoResponse::from_key_with_rate_limit(
        &key,
        payload.rate_limit_rpm,
        deprecated_at,
    )))
}

/// Rotate an API key: creates a new key and deprecates the old one.
///
/// The old key remains usable during the grace window (default 48h,
/// configurable via `API_KEY_DEPRECATION_GRACE_SECS`).
#[utoipa::path(
    post,
    path = "/users/api-keys/{id}/rotate",
    tag = "users",
    security(("bearer_auth" = [])),
    params(
        ("id" = Uuid, Path, description = "API key ID to rotate"),
    ),
    responses(
        (status = 201, description = "Key rotated", body = RotateApiKeyResponsePayload),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "API key not found"),
        (status = 409, description = "Key already deprecated or inactive"),
    )
)]
pub async fn rotate_api_key<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<RotateApiKeyResponsePayload>), StatusCode>
where
    A: SessionService + 'static,
{
    // Fetch the existing key via trait (for ownership + name)
    let key = state
        .data_service
        .get_api_key(ApiKeyId(id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Verify ownership
    if key.user_id != user.id {
        return Err(StatusCode::NOT_FOUND);
    }

    // Must be active
    if !key.is_active {
        return Err(StatusCode::CONFLICT);
    }

    // Check deprecation via PgDataService (the trait model doesn't have deprecated_at)
    let auth_info = state
        .data_service
        .get_api_key_auth_info_by_id(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if auth_info.deprecated_at.is_some() {
        return Err(StatusCode::CONFLICT);
    }

    // Create the replacement key
    let new_name = format!("{} (rotated)", key.name);
    let (raw_key, new_api_key) = build_api_key(&new_name, user.id, key.expires_at);

    state
        .data_service
        .create_api_key(&new_api_key)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Deprecate the old key
    let now = Utc::now();
    state
        .data_service
        .set_api_key_deprecated(id, now)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok((
        StatusCode::CREATED,
        Json(RotateApiKeyResponsePayload {
            id: new_api_key.id.0,
            name: new_name,
            key_prefix: new_api_key.key_prefix,
            created_at: new_api_key.created_at,
            key: raw_key,
            old_key_deprecated_at: now,
        }),
    ))
}

// === Helpers ===

/// Build a new ApiKey struct and return (plaintext, model).
fn build_api_key(
    name: &str,
    user_id: auth::UserId,
    expires_at: Option<DateTime<Utc>>,
) -> (String, ApiKey) {
    let raw_key = format!(
        "ak_{}_{}",
        generate_key_segment(4),
        generate_key_segment(32)
    );
    let key_prefix = format!("{}****{}", &raw_key[..8], &raw_key[raw_key.len() - 4..]);
    let key_hash = hash_api_key(&raw_key);
    let now = Utc::now();

    let api_key = ApiKey {
        id: ApiKeyId::new(),
        user_id,
        name: name.to_string(),
        key_hash,
        key_prefix,
        is_active: true,
        created_at: now,
        last_used_at: None,
        expires_at,
    };

    (raw_key, api_key)
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
