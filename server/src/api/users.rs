//! User API endpoints — API key management.
//!
//! All endpoints require authentication via session token or API key.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use auth::{ApiKey, ApiKeyId, ApiKeyRepository, SessionService};

use super::extractors::AuthenticatedUser;
use crate::state::PgAppState;

/// Response for listing API keys.
#[derive(Debug, Serialize, ToSchema)]
pub struct ApiKeyListResponse {
    pub keys: Vec<ApiKeyInfoResponse>,
}

/// API key info for list/get responses (includes deprecation status).
#[derive(Debug, Serialize, ToSchema)]
pub struct ApiKeyInfoResponse {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    /// When this key was deprecated (rotated). Null for active keys.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated_at: Option<DateTime<Utc>>,
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
        .list_user_api_keys_full(user.id.0)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let keys = keys
        .into_iter()
        .map(|k| ApiKeyInfoResponse {
            id: k.id,
            name: k.name,
            key_prefix: k.key_prefix,
            is_active: k.is_active,
            created_at: k.created_at,
            last_used_at: k.last_used_at,
            expires_at: k.expires_at,
            deprecated_at: k.deprecated_at,
        })
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

/// Generate a random hex segment for API key generation.
fn generate_key_segment(bytes: usize) -> String {
    use std::fmt::Write;
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("getrandom failed");
    let mut s = String::with_capacity(bytes * 2);
    for b in &buf {
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

/// Response after rotating an API key (includes the new plaintext key).
#[derive(Debug, Serialize, ToSchema)]
pub struct RotateApiKeyResponsePayload {
    /// ID of the new (replacement) key.
    pub key_id: Uuid,
    /// The plaintext API key. Store this securely — it cannot be retrieved again.
    pub raw_key: String,
}

/// Rotate an API key: generates a new key, deprecates the old one.
///
/// The old key remains valid for a grace window (default 48 h, controlled by
/// `API_KEY_DEPRECATION_GRACE_SECS`). After the grace window it stops
/// authenticating.
#[utoipa::path(
    post,
    path = "/users/api-keys/{id}/rotate",
    tag = "users",
    security(("bearer_auth" = [])),
    params(
        ("id" = Uuid, Path, description = "API key ID to rotate"),
    ),
    responses(
        (status = 200, description = "Key rotated", body = RotateApiKeyResponsePayload),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "API key not found"),
    )
)]
pub async fn rotate_api_key<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(id): Path<Uuid>,
) -> Result<Json<RotateApiKeyResponsePayload>, StatusCode>
where
    A: SessionService + 'static,
{
    // Verify the key belongs to this user and is active
    let key = state
        .data_service
        .get_api_key(ApiKeyId(id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if key.user_id != user.id {
        return Err(StatusCode::NOT_FOUND);
    }
    if !key.is_active {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Generate a new API key with the same generation logic
    let raw_key = format!(
        "ak_{}_{}",
        generate_key_segment(4),
        generate_key_segment(32)
    );
    let key_prefix = format!("{}****{}", &raw_key[..8], &raw_key[raw_key.len() - 4..]);
    let key_hash = hash_api_key(&raw_key);

    let now = Utc::now();
    let new_key = ApiKey {
        id: ApiKeyId::new(),
        user_id: user.id,
        name: format!("{} (rotated)", key.name),
        key_hash,
        key_prefix,
        is_active: true,
        created_at: now,
        last_used_at: None,
        expires_at: key.expires_at,
    };

    // Create the new key
    state
        .data_service
        .create_api_key(&new_key)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Deprecate the old key
    state
        .data_service
        .set_api_key_deprecated(id, now)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(RotateApiKeyResponsePayload {
        key_id: new_key.id.0,
        raw_key,
    }))
}
