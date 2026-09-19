//! User API endpoints — API key management.
//!
//! All endpoints require authentication via session token.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use sha3::Digest;
use uuid::Uuid;

use auth::{
    ApiKey, ApiKeyId, ApiKeyInfo, ApiKeyRepository, Permission, Role, SessionService,
    WalletCredential, WalletCredentialId, WalletRepository,
};
use data_service::ApiKeyFullInfo;

use super::api_key_hash::hash_api_key;
use super::extractors::{AuthenticatedUser, FreshlyAuthenticatedUser};
use crate::services::EmailChangeVerificationData;
use crate::state::PgAppState;
pub use api_types::UpdateApiKeyPayload;

/// API key info for list/get responses.
///
/// Hand-mirrors `api_types::ApiKeyInfoResponse` with one added field
/// (`permissions`) rather than extending that pinned struct: `api-types`
/// lives in payserver-commons, and landing a field there is the three-step
/// dance (merge, bump the pinned rev, `cargo update`) this repo's
/// `CLAUDE.md` describes - a cross-repo change this ticket cannot complete
/// on its own. Same reasoning as `WalletCredentialResponse` below. The wire
/// shape only grows a field, so a client still built against the pinned
/// type keeps working unchanged.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
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
    /// When the grace window ends for a deprecated key. Null for
    /// non-deprecated keys.
    pub deprecation_expires_at: Option<DateTime<Utc>>,
    /// Permission policy strings this key is scoped to. `None` means it
    /// inherits its owner's role in full - either because it predates this
    /// column, or because nobody has narrowed it since.
    pub permissions: Option<Vec<String>>,
}

/// Response for listing API keys.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ApiKeyListResponse {
    pub keys: Vec<ApiKeyInfoResponse>,
}

/// Request to create a new API key. See `ApiKeyInfoResponse` for why this is
/// hand-mirrored rather than extending the pinned `api_types` struct.
#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateApiKeyPayload {
    /// Human-readable name for the key.
    pub name: String,
    /// Optional expiration time.
    pub expires_at: Option<DateTime<Utc>>,
    /// The key's scope, chosen at creation. Only two shapes are accepted
    /// today: `[]` (scoped to the owner's non-admin baseline) or
    /// `["unrestricted"]` (see `auth::Policies::UNRESTRICTED`, inherits the
    /// owner's role in full). Defaults to `[]` when omitted: a new key
    /// starts able to do nothing beyond authenticating and must be
    /// deliberately widened, rather than silently inheriting everything its
    /// owner can do. `unrestricted` is only accepted when the caller's own
    /// current role grants it - a key can never exceed its owner, including
    /// at the moment it is minted. See `validate_requested_permissions` for
    /// why the vocabulary stops at these two shapes rather than the full
    /// list of named policies: nothing in this server enforces those
    /// individually yet, so offering a menu implying otherwise would be
    /// worse than not offering it.
    #[serde(default)]
    pub permissions: Vec<String>,
}

/// Response after creating an API key (includes plaintext key).
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct CreateApiKeyResponsePayload {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    /// The plaintext API key. Store this securely — it cannot be retrieved again.
    pub key: String,
    pub permissions: Vec<String>,
}

/// Response after rotating an API key.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
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
    /// When the old key's grace window ends and it stops authenticating.
    /// Clients should show this directly instead of hardcoding "48 hours".
    pub old_key_grace_expires_at: DateTime<Utc>,
    /// Permission scope carried over from the key being rotated.
    pub permissions: Option<Vec<String>>,
}

/// Body for `PATCH /users/api-keys/{id}/permissions`.
#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct UpdateApiKeyPermissionsPayload {
    /// `None`/`null` clears the key back to "inherit the owner's role in
    /// full". `Some` sets an explicit scope - see `CreateApiKeyPayload`'s
    /// `permissions` field for the two shapes accepted and why.
    pub permissions: Option<Vec<String>>,
}

/// Is `requested` one of the two scopes this server can actually enforce?
///
/// Every admin gate in this codebase (there are over a dozen, from plugin
/// install to user role management) is a bare `role == Role::ServerAdmin`
/// comparison, not a check against an individual `Permission` - see
/// `validate_api_key`'s downgrade and `key_retains_unrestricted_access` in
/// `extractors.rs`. So a stored set naming specific server or user policies
/// (e.g. just `ethpay.server.canmanagetokens`) would be silently
/// indistinguishable from an empty one: neither grants anything past a plain
/// `User`'s fixed permissions. Offering a menu of individually-named actions
/// that all collapse to the same outcome is worse than not offering it, so
/// this only accepts what the server can actually tell apart: nothing beyond
/// the owner's non-admin baseline (`[]`), or the owner's full role
/// (`["unrestricted"]`). Widening this to real per-action scoping needs
/// per-endpoint enforcement this codebase does not have yet, not a change
/// here.
///
/// `owner_role` is always the role belonging to the account the key
/// authenticates as, never the caller's - the write-time half of "a key can
/// never exceed its owner". Read-time enforcement (`validate_api_key`, and
/// every `role == Role::ServerAdmin` check downstream of it) covers a key
/// whose owner's role later changes, but without this an admin editing
/// someone else's key could grant it `unrestricted` on the strength of the
/// *admin's* role, which that key's own owner could never grant themselves -
/// precisely the "promoting the owner must not widen keys already issued"
/// failure this feature exists to close, from the other direction.
fn validate_requested_permissions(owner_role: Role, requested: &[String]) -> Result<(), StatusCode> {
    match requested {
        [] => Ok(()),
        [single] if single == Permission::Unrestricted.as_policy() => {
            if owner_role == Role::ServerAdmin {
                Ok(())
            } else {
                Err(StatusCode::BAD_REQUEST)
            }
        }
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

#[cfg(test)]
mod permission_scope_tests {
    use super::*;

    #[test]
    fn an_empty_scope_is_always_accepted() {
        assert!(validate_requested_permissions(Role::User, &[]).is_ok());
        assert!(validate_requested_permissions(Role::ServerAdmin, &[]).is_ok());
    }

    #[test]
    fn unrestricted_is_accepted_only_for_a_server_admin_owner() {
        let unrestricted = vec![Permission::Unrestricted.as_policy().to_string()];
        assert!(validate_requested_permissions(Role::ServerAdmin, &unrestricted).is_ok());
        assert!(validate_requested_permissions(Role::User, &unrestricted).is_err());
    }

    #[test]
    fn a_named_individual_permission_is_rejected_even_for_an_admin_owner() {
        // Nothing in this server enforces named permissions individually -
        // accepting one here would promise scoping the rest of the codebase
        // cannot deliver.
        let named = vec![Permission::ServerManageTokens.as_policy().to_string()];
        assert!(validate_requested_permissions(Role::ServerAdmin, &named).is_err());
    }

    #[test]
    fn unrestricted_mixed_with_anything_else_is_rejected() {
        let mixed = vec![
            Permission::Unrestricted.as_policy().to_string(),
            Permission::ServerViewUsers.as_policy().to_string(),
        ];
        assert!(validate_requested_permissions(Role::ServerAdmin, &mixed).is_err());
    }
}

/// Build from an `ApiKey` plus the ancillary rate-limit / deprecation /
/// permission fields not present on the auth-crate struct. Used by
/// endpoints that already have an `ApiKey` in hand (e.g. update_api_key
/// after a mutation).
pub(crate) fn api_key_info_with_rate_limit(
    key: &ApiKey,
    rate_limit_rpm: Option<i32>,
    deprecated_at: Option<DateTime<Utc>>,
    permissions: Option<Vec<String>>,
) -> ApiKeyInfoResponse {
    let info = ApiKeyInfo::from(key);
    ApiKeyInfoResponse {
        id: info.id.0,
        name: info.name,
        key_prefix: info.key_prefix,
        is_active: info.is_active,
        created_at: info.created_at,
        last_used_at: info.last_used_at,
        expires_at: info.expires_at,
        rate_limit_rpm,
        deprecated_at,
        deprecation_expires_at: deprecated_at.map(deprecation_expires_at),
        permissions,
    }
}

/// Build the wire shape from the `auth` domain type.
///
/// A free function rather than a `From` impl: `ApiKeyFullInfo` belongs to
/// `data_service` and `ApiKeyInfoResponse` is local to this module (see its
/// doc comment), so neither owns the other.
pub(crate) fn api_key_info_response(info: ApiKeyFullInfo) -> ApiKeyInfoResponse {
    ApiKeyInfoResponse {
        id: info.id,
        name: info.name,
        key_prefix: info.key_prefix,
        is_active: info.is_active,
        created_at: info.created_at,
        last_used_at: info.last_used_at,
        expires_at: info.expires_at,
        rate_limit_rpm: info.rate_limit_rpm,
        deprecated_at: info.deprecated_at,
        deprecation_expires_at: info.deprecated_at.map(deprecation_expires_at),
        permissions: info.permissions,
    }
}

/// Translate a `deprecated_at` into the grace-window deadline. Uses the same
/// grace-seconds value as the auth extractor, so the client-visible expiry
/// matches when the server actually starts rejecting the key.
fn deprecation_expires_at(deprecated_at: DateTime<Utc>) -> DateTime<Utc> {
    deprecated_at + chrono::Duration::seconds(super::extractors::deprecation_grace_secs())
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

    let keys = keys.into_iter().map(api_key_info_response).collect();

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

    validate_requested_permissions(user.role, &payload.permissions)?;

    let (raw_key, api_key) = build_api_key(&name, user.id, payload.expires_at);

    state
        .data_service
        .create_api_key_with_permissions(&api_key, Some(&payload.permissions))
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
            permissions: payload.permissions,
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

    // Preserve any existing deprecation/permission state in the response
    // rather than always returning None — prevents a stale-UI bug where the
    // client thinks the key was un-deprecated, or reset to full access,
    // after a rate-limit update.
    let auth_info = state
        .data_service
        .get_api_key_auth_info_by_id(id)
        .await
        .ok()
        .flatten();
    let deprecated_at = auth_info.as_ref().and_then(|info| info.deprecated_at);
    let permissions = auth_info.and_then(|info| info.permissions);

    Ok(Json(api_key_info_with_rate_limit(
        &key,
        payload.rate_limit_rpm,
        deprecated_at,
        permissions,
    )))
}

/// Narrow (or, for a still-fully-privileged caller, widen back to
/// "inherit") an existing API key's permission scope.
///
/// A separate endpoint from `update_api_key` rather than folded into it:
/// that endpoint always overwrites every field in its body, and a caller
/// that only wants to change the rate limit must not be able to
/// accidentally reset a deliberately narrowed key back to full access just
/// by omitting a field it does not know exists.
///
/// Clearing to "inherit" (`permissions: null`) is refused unless the caller
/// is unrestricted for *this* request - not merely `role == ServerAdmin` on
/// the account, but actually holding that role right now. A key that has
/// itself been scoped away from `unrestricted` authenticates as a plain
/// `User` (see `validate_api_key`), so it cannot use this endpoint to hand
/// itself back the access it was narrowed away from, even against its own
/// row.
#[utoipa::path(
    patch,
    path = "/users/api-keys/{id}/permissions",
    tag = "users",
    security(("bearer_auth" = [])),
    params(
        ("id" = Uuid, Path, description = "API key ID to update"),
    ),
    request_body = UpdateApiKeyPermissionsPayload,
    responses(
        (status = 200, description = "Permission scope updated", body = ApiKeyInfoResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "API key not found"),
    )
)]
pub async fn update_api_key_permissions<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateApiKeyPermissionsPayload>,
) -> Result<Json<ApiKeyInfoResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let key = state
        .data_service
        .get_api_key(ApiKeyId(id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if key.user_id != user.id && user.role != Role::ServerAdmin {
        return Err(StatusCode::NOT_FOUND);
    }

    // The role to validate a widened grant against is the key's own owner's,
    // not the caller's - an admin editing someone else's key must not be
    // able to launder a grant through their own broader role. Only fetched
    // when the caller isn't the owner: the common case (a user managing
    // their own key) already has this in hand.
    let owner_role = if key.user_id == user.id {
        user.role
    } else {
        auth::UserRepository::get_user(&*state.data_service, key.user_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?
            .role
    };

    let permissions = match &payload.permissions {
        Some(requested) => {
            validate_requested_permissions(owner_role, requested)?;
            Some(requested.clone())
        }
        None => {
            if user.role != Role::ServerAdmin {
                return Err(StatusCode::BAD_REQUEST);
            }
            None
        }
    };

    state
        .data_service
        .update_api_key_permissions(ApiKeyId(id), permissions.as_deref())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let auth_info = state
        .data_service
        .get_api_key_auth_info_by_id(id)
        .await
        .ok()
        .flatten();
    let (rate_limit_rpm, deprecated_at) = auth_info
        .map(|info| (info.rate_limit_rpm, info.deprecated_at))
        .unwrap_or_default();

    Ok(Json(api_key_info_with_rate_limit(
        &key,
        rate_limit_rpm,
        deprecated_at,
        permissions,
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

    // Create the replacement key + deprecate the old one atomically.
    // Without a transaction a partial failure (new key created, deprecation
    // fails) would leave TWO active keys on the account — the explicit
    // enemy of rotation.
    //
    // The new key carries over the old key's permission scope: rotation
    // swaps the secret, it does not widen what the key can do.
    let new_name = format!("{} (rotated)", key.name);
    let (raw_key, new_api_key) = build_api_key(&new_name, user.id, key.expires_at);
    let now = Utc::now();

    state
        .data_service
        .rotate_api_key_atomic(&new_api_key, auth_info.permissions.as_deref(), id, now)
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
            old_key_grace_expires_at: deprecation_expires_at(now),
            permissions: auth_info.permissions,
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

// =========================================================================
// Wallet credentials (login identity)
//
// SENSITIVE: this section changes and lists login-credential wallets and the
// primary-wallet pointer wallet login resolves accounts by. Human review
// required without exception.
// =========================================================================

/// A wallet credential, as returned to the account owner.
///
/// Hand-mirrors `auth::WalletInfo` rather than reusing it directly: `auth` is
/// a server-side crate the client does not depend on (see
/// `api_key_info_response` above for the same reasoning), and this DTO can't
/// be added to the shared `api-types` crate in this change — that crate is
/// pinned by `rev` in a sibling repository and moving the pin is its own
/// three-step change. The client defines a matching struct for deserializing
/// this response.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct WalletCredentialResponse {
    pub id: Uuid,
    pub address: String,
    pub name: String,
    pub is_primary: bool,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

impl From<WalletCredential> for WalletCredentialResponse {
    fn from(w: WalletCredential) -> Self {
        Self {
            id: w.id.0,
            address: w.address,
            name: w.name,
            is_primary: w.is_primary,
            created_at: w.created_at,
            last_used_at: w.last_used_at,
        }
    }
}

/// List the authenticated account's wallet login credentials.
///
/// A plain valid session is enough to *read* this — it is the same
/// information `GET /auth/wallets` already returns. Only the write below
/// (making one of them primary) is gated on a fresh re-authentication.
#[utoipa::path(
    get,
    path = "/users/wallets",
    tag = "users",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Wallet credentials for this account", body = Vec<WalletCredentialResponse>),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn list_wallet_credentials<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
) -> Result<Json<Vec<WalletCredentialResponse>>, StatusCode>
where
    A: SessionService + 'static,
{
    let mut wallets = state
        .data_service
        .get_wallets_for_user(user.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    wallets.retain(|w| w.is_active);

    Ok(Json(
        wallets
            .into_iter()
            .map(WalletCredentialResponse::from)
            .collect(),
    ))
}

/// How long a wallet-reauth challenge stays answerable. Generous enough to
/// unlock a wallet extension and approve its signing prompt, short enough
/// that an unused challenge does not linger as a standing credential. Also
/// enforced in SQL by `take_wallet_reauth_challenge`, which is what the
/// endpoints below actually rely on — this constant only has to agree with
/// that query, not re-derive it.
const WALLET_REAUTH_CHALLENGE_TTL_SECS: i64 = 5 * 60;

/// Build the message a wallet extension shows in its signing prompt for a
/// reauth challenge.
///
/// Deliberately worded differently from the auth crate's own login-challenge
/// message ("Sign this message to authenticate to..."): the two ceremonies
/// use separate challenge storage (`wallet_reauth_challenges` vs.
/// `wallet_challenges`) and must never be interchangeable, so the text a
/// merchant is asked to sign should look different too, not just hash
/// differently.
fn wallet_reauth_challenge_message(
    challenge: &str,
    address: &str,
    created_at: DateTime<Utc>,
) -> String {
    format!(
        "Confirm this wallet change on random.cash:\n\nChallenge: {challenge}\nTimestamp: {}\nAddress: {address}",
        created_at.to_rfc3339()
    )
}

/// Verify an EIP-191 `personal_sign` signature recovers to `expected_address`.
///
/// Hand-mirrors `auth::service::wallet::verify_wallet_signature`: that
/// function (and the EIP-191/Keccak256/ECDSA-recovery it does) lives
/// `pub(super)` inside the pinned `auth` crate in payserver-commons, not
/// reachable from here, and making it public is its own three-step commons
/// change (merge there, bump the pinned rev, `cargo update`) this ticket
/// does not need — the challenge this checks is entirely local to
/// `wallet_reauth_challenges` and never mixes with `auth`'s own login
/// challenges. Same recovery arithmetic, independent code path.
fn verify_wallet_signature(message: &str, signature_hex: &str, expected_address: &str) -> bool {
    let Ok(signature_bytes) = hex::decode(signature_hex.trim_start_matches("0x")) else {
        return false;
    };
    if signature_bytes.len() != 65 {
        return false;
    }
    let (r_s, v) = signature_bytes.split_at(64);
    let Ok(signature) = k256::ecdsa::Signature::from_slice(r_s) else {
        return false;
    };
    let recovery_id = match v[0] {
        27 | 0 => k256::ecdsa::RecoveryId::try_from(0u8),
        28 | 1 => k256::ecdsa::RecoveryId::try_from(1u8),
        _ => return false,
    };
    let Ok(recovery_id) = recovery_id else {
        return false;
    };

    let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
    let mut hasher = sha3::Keccak256::new();
    hasher.update(prefix.as_bytes());
    hasher.update(message.as_bytes());
    let message_hash = hasher.finalize();

    let Ok(recovered_key) =
        k256::ecdsa::VerifyingKey::recover_from_prehash(&message_hash, &signature, recovery_id)
    else {
        return false;
    };

    let public_key_bytes = recovered_key.to_encoded_point(false);
    let public_key_hash = sha3::Keccak256::digest(&public_key_bytes.as_bytes()[1..]);
    let recovered_address = format!("0x{}", hex::encode(&public_key_hash[12..]));

    recovered_address.eq_ignore_ascii_case(expected_address)
}

/// What the caller must sign to prove they currently hold the private key
/// for the wallet credential they are about to promote.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct WalletReauthChallengeResponse {
    pub message: String,
    pub expires_in_secs: i64,
}

/// Request a proof-of-possession challenge for wallet credential `id`.
///
/// Step one of promoting a wallet to primary. A session alone — even a
/// freshly-minted one — proves who is logged in, not that the caller still
/// controls the address being promoted to a login credential: a hijacked
/// session has no notion of "just logged in" strong enough to rule that out
/// on its own. This challenge, and the signature `PATCH .../primary` below
/// requires against it, ask for the one thing a session hijack cannot
/// forge — a fresh signature from the wallet's own key.
#[utoipa::path(
    post,
    path = "/users/wallets/{id}/reauth-challenge",
    tag = "users",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Wallet credential to prove current ownership of")),
    responses(
        (status = 200, description = "Challenge issued", body = WalletReauthChallengeResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "No such active wallet credential on this account"),
    )
)]
pub async fn create_wallet_reauth_challenge<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(id): Path<Uuid>,
) -> Result<Json<WalletReauthChallengeResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let wallet = state
        .data_service
        .get_wallet(WalletCredentialId(id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .filter(|w| w.user_id == user.id && w.is_active)
        .ok_or(StatusCode::NOT_FOUND)?;

    let challenge = generate_key_segment(32);
    let created_at = Utc::now();

    state
        .data_service
        .store_wallet_reauth_challenge(user.id, &wallet.address, &challenge, created_at)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(WalletReauthChallengeResponse {
        message: wallet_reauth_challenge_message(&challenge, &wallet.address, created_at),
        expires_in_secs: WALLET_REAUTH_CHALLENGE_TTL_SECS,
    }))
}

/// Body for `PATCH /users/wallets/{id}/primary`: the signature over the
/// challenge issued by `POST /users/wallets/{id}/reauth-challenge`.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct PromoteWalletCredentialRequest {
    pub signature: String,
}

/// Make an existing wallet credential the account's primary — the address
/// wallet login resolves the account by, and the one shown in Settings.
///
/// Requires both a valid session (`AuthenticatedUser`) and a signature over
/// a fresh `POST .../reauth-challenge` issued for this exact wallet — proof
/// that the caller controls the address right now, not just that they proved
/// it once at `complete_wallet_registration` time and are currently carrying
/// a session cookie. The two checks cover different attackers: the session
/// rules out an anonymous caller, the signature rules out a hijacked session
/// that never held the wallet's key.
///
/// Deliberately does not accept a bare address plus signature for a brand
/// new address. `id` must already name an active `WalletCredential`
/// belonging to this account — ownership of that address was already proven
/// once, via the existing challenge/signature flow, when it was added
/// (`complete_wallet_registration`) or at account creation. Accepting an
/// unregistered address here instead would let this endpoint be used to
/// register a credential outside that flow.
#[utoipa::path(
    patch,
    path = "/users/wallets/{id}/primary",
    tag = "users",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Wallet credential to make primary")),
    request_body = PromoteWalletCredentialRequest,
    responses(
        (status = 200, description = "Primary wallet changed", body = WalletCredentialResponse),
        (status = 401, description = "Unauthorized, or no valid reauth signature for this wallet"),
        (status = 404, description = "No such active wallet credential on this account"),
    )
)]
pub async fn set_primary_wallet_credential<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<PromoteWalletCredentialRequest>,
) -> Result<Json<WalletCredentialResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let wallet = state
        .data_service
        .get_wallet(WalletCredentialId(id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .filter(|w| w.user_id == user.id && w.is_active)
        .ok_or(StatusCode::NOT_FOUND)?;

    let challenge = state
        .data_service
        .take_wallet_reauth_challenge(user.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // The challenge must have been issued for exactly this wallet's address —
    // a challenge answered for a different credential must not authorize
    // promoting this one.
    if !challenge.address.eq_ignore_ascii_case(&wallet.address) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let expected_message = wallet_reauth_challenge_message(
        &challenge.challenge,
        &challenge.address,
        challenge.created_at,
    );
    if !verify_wallet_signature(&expected_message, &payload.signature, &challenge.address) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    state
        .data_service
        .set_primary_wallet_credential(user.id, WalletCredentialId(id))
        .await
        .map(|w| Json(WalletCredentialResponse::from(w)))
        .map_err(|e| match e {
            auth::AuthError::WalletNotFound(_) => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        })
}

#[cfg(test)]
mod wallet_reauth_tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    // Deterministic 32-byte private key, matching the pattern the auth
    // crate's own `test_wallet_signature_verification` uses — not a real
    // key, chosen only so both the test and a would-be attacker can derive
    // the same address from it.
    const TEST_PRIVATE_KEY: [u8; 32] = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e,
        0x1f, 0x20,
    ];

    fn test_signing_key() -> k256::ecdsa::SigningKey {
        k256::ecdsa::SigningKey::from_bytes((&TEST_PRIVATE_KEY).into()).unwrap()
    }

    fn address_for(signing_key: &k256::ecdsa::SigningKey) -> String {
        let verifying_key = signing_key.verifying_key();
        let public_key_bytes = verifying_key.to_encoded_point(false);
        let public_key_hash = sha3::Keccak256::digest(&public_key_bytes.as_bytes()[1..]);
        format!("0x{}", hex::encode(&public_key_hash[12..]))
    }

    fn sign(signing_key: &k256::ecdsa::SigningKey, message: &str) -> String {
        let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
        let mut hasher = sha3::Keccak256::new();
        hasher.update(prefix.as_bytes());
        hasher.update(message.as_bytes());
        let message_hash = hasher.finalize();

        let (signature, recovery_id) = signing_key.sign_prehash_recoverable(&message_hash).unwrap();
        let mut sig_bytes = signature.to_bytes().to_vec();
        sig_bytes.push(recovery_id.to_byte() + 27);
        format!("0x{}", hex::encode(&sig_bytes))
    }

    #[test]
    fn a_correctly_signed_challenge_verifies() {
        let key = test_signing_key();
        let address = address_for(&key);
        let message = wallet_reauth_challenge_message("deadbeef", &address, Utc::now());
        let signature = sign(&key, &message);

        assert!(verify_wallet_signature(&message, &signature, &address));
    }

    #[test]
    fn a_signature_over_a_different_message_does_not_verify() {
        let key = test_signing_key();
        let address = address_for(&key);
        let message = wallet_reauth_challenge_message("deadbeef", &address, Utc::now());
        let signature = sign(&key, &message);

        let tampered = wallet_reauth_challenge_message("deadc0de", &address, Utc::now());
        assert!(!verify_wallet_signature(&tampered, &signature, &address));
    }

    #[test]
    fn a_signature_from_a_different_key_does_not_verify() {
        let key = test_signing_key();
        let address = address_for(&key);
        let message = wallet_reauth_challenge_message("deadbeef", &address, Utc::now());
        let signature = sign(&key, &message);

        // Someone who hijacked the session but does not hold the wallet's
        // private key cannot produce a signature that recovers to it — this
        // is the property that makes the challenge a genuine step-up rather
        // than just a session check.
        let other_address = "0x000000000000000000000000000000000000ff";
        assert!(!verify_wallet_signature(
            &message,
            &signature,
            other_address
        ));
    }

    #[test]
    fn garbage_signature_hex_does_not_verify() {
        let message = wallet_reauth_challenge_message("deadbeef", "0xabc", Utc::now());
        assert!(!verify_wallet_signature(&message, "not-hex", "0xabc"));
        assert!(!verify_wallet_signature(&message, "0x1234", "0xabc"));
    }
}

// =========================================================================
// Account deletion
// =========================================================================

/// Confirmation the caller must type back before the account is deleted.
///
/// A query parameter rather than a JSON body, deliberately: a shared body type
/// would have to live in `api-types` and be mirrored by the client, and a DTO
/// the two sides define separately is the drift this codebase already paid for
/// once. There is nothing secret in it - it is the account's own email or id,
/// both of which the caller must already be authenticated to know.
#[derive(Debug, serde::Deserialize)]
pub struct DeleteAccountQuery {
    /// Must equal the account's email, or its id when there is no email.
    pub confirm: String,
}

/// What the caller must type to confirm.
///
/// Email where the account has one, because it is the handle a merchant knows.
/// A passkey-only account has no email and no wallet, so its id is the only
/// thing it can be asked for - the same reason recovery accepts a UUID.
fn deletion_confirmation_for(user: &auth::UserInfo) -> String {
    user.email.clone().unwrap_or_else(|| user.id.0.to_string())
}

/// Whether what was typed matches, ignoring case and surrounding whitespace.
///
/// Case-insensitive because email is, and a merchant retyping their own address
/// with a capital letter has still proved intent. Not a secret comparison, so
/// there is nothing to keep constant-time.
fn deletion_confirmation_matches(expected: &str, typed: &str) -> bool {
    typed.trim().eq_ignore_ascii_case(expected.trim())
}

/// Delete the authenticated account.
///
/// Refuses while the account's stores hold any payment, payout or refund. That
/// is not squeamishness: `users` cascades through `stores` into `invoices` and
/// `payments`, so deleting a merchant who traded would erase the records they
/// need to answer a customer, a chargeback or a tax question - and `payouts`
/// and `refunds` are `NO ACTION`, so the same delete would fail on a foreign
/// key and surface as a 500. `DELETE /stores/{id}` already archives rather than
/// deletes for this reason; this endpoint declines rather than pretending.
///
/// What it is for is the case deletion is actually asked for: an abandoned
/// signup, a test account, a merchant who never traded. Those cascade cleanly -
/// devices, sessions, passkeys, api keys, wallets, empty stores - and leave
/// nothing behind.
#[utoipa::path(
    delete,
    path = "/users/me",
    params(("confirm" = String, Query, description = "The account's email, or its id when it has no email")),
    responses(
        (status = 204, description = "Account deleted"),
        (status = 400, description = "Confirmation did not match"),
        (status = 409, description = "Account holds financial records and cannot be deleted"),
    ),
    tag = "users"
)]
pub async fn delete_account<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    axum::extract::Query(query): axum::extract::Query<DeleteAccountQuery>,
) -> Result<StatusCode, (StatusCode, String)>
where
    A: SessionService + 'static,
{
    let expected = deletion_confirmation_for(&user);
    if !deletion_confirmation_matches(&expected, &query.confirm) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Confirmation did not match this account.".to_string(),
        ));
    }

    let blockers = data_service::AccountDeletionReader::account_deletion_blockers(
        &*state.data_service,
        user.id,
    )
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not check the account.".to_string(),
        )
    })?;

    if blockers.any() {
        // Named, not a bare refusal: a merchant who cannot delete needs to know
        // what is holding it, and an operator triaging this needs the same.
        return Err((
            StatusCode::CONFLICT,
            format!(
                "This account's stores hold {}. Deleting it would destroy that \
                 history, so it is refused. Archive the stores instead.",
                blockers.describe()
            ),
        ));
    }

    auth::UserRepository::delete_user(&*state.data_service, user.id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not delete the account.".to_string(),
            )
        })?;

    tracing::info!(user_id = %user.id.0, "account deleted at its owner's request");
    Ok(StatusCode::NO_CONTENT)
}

// =========================================================================
// Email change
//
// SENSITIVE: this touches account recovery. Set, change and remove all sit
// behind `FreshlyAuthenticatedUser` (server/src/api/extractors.rs) rather than
// `AuthenticatedUser` - a session hijacked hours into its lifetime must not be
// able to swap the account's recovery address. None of this touches
// `kdf_salt_identifier`: it is set once at registration and is immutable
// (enforced in `auth::UserRepository::update_user`), and the recovery KDF
// stays salted with whatever identifier was pinned then regardless of what
// `email` becomes later. That is what makes changing email safe at all.
// =========================================================================

/// How long a pending email change's verification code remains redeemable.
const EMAIL_CHANGE_TOKEN_TTL: chrono::Duration = chrono::Duration::minutes(30);

/// Body for `POST /users/me/email`. Covers both setting an email where there
/// was none and changing an existing one - the server treats them identically,
/// since either way the new address must be verified before it lands.
///
/// Defined here rather than in `api-types`: that crate is where a JSON body
/// normally belongs so the client shares the exact shape instead of a
/// hand-mirrored copy (see its module doc - a drifted copy has broken the
/// client on contact with the API before). But `api-types` lives in
/// `payserver-commons`, and landing a change there is the three-step dance
/// this repo's `CLAUDE.md` describes: merge in commons, bump the pinned `rev`
/// here, only then does the code see it - a cross-repo review this single
/// ticket cannot complete on its own. `DeleteAccountQuery` took the same
/// exception for the same reason. The shape is two primitive fields, so the
/// drift risk is small and worth accepting rather than blocking this ticket
/// on an external merge.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct RequestEmailChangePayload {
    pub new_email: String,
}

/// Body for `POST /users/me/email/confirm`. See `RequestEmailChangePayload`
/// for why this is local rather than in `api-types`.
///
/// Deliberately unauthenticated: the token itself, delivered only to the
/// address being verified, is the proof. Requiring a session on top would add
/// nothing but a way for a merchant who requested the change from one browser
/// to be unable to confirm it from another (e.g. opening the email on a
/// phone).
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct ConfirmEmailChangePayload {
    pub token: Uuid,
}

/// Very small email-shape check.
///
/// `auth::service::validation::validate_email` does the same job but is
/// private to that crate, so this mirrors its rules rather than pulling in
/// the whole auth crate's validation surface for one check. Not exhaustive -
/// just enough to reject an obviously wrong address before minting a token
/// and an email for it.
fn looks_like_an_email(email: &str) -> bool {
    let email = email.trim();
    let parts: Vec<&str> = email.split('@').collect();
    let [local, domain] = parts.as_slice() else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
}

/// Start changing the authenticated account's email.
///
/// Requires a fresh passkey or wallet login (see `FreshlyAuthenticatedUser`),
/// not merely a valid session. Fails loudly rather than queuing a change
/// nobody can confirm: if SMTP is not configured on this server, the address
/// would sit pending forever with no error anywhere, which is exactly the
/// no-op-by-design behaviour that is right for a payment receipt and wrong
/// here.
#[utoipa::path(
    post,
    path = "/users/me/email",
    tag = "users",
    security(("bearer_auth" = [])),
    request_body = RequestEmailChangePayload,
    responses(
        (status = 202, description = "Verification email sent"),
        (status = 400, description = "Invalid email address"),
        (status = 401, description = "Unauthorized, or session is not fresh enough"),
        (status = 409, description = "Email already in use"),
        (status = 503, description = "Email is not configured on this server"),
    )
)]
pub async fn request_email_change<A>(
    FreshlyAuthenticatedUser(user): FreshlyAuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Json(payload): Json<RequestEmailChangePayload>,
) -> Result<StatusCode, (StatusCode, String)>
where
    A: SessionService + 'static,
{
    let new_email = payload.new_email.trim().to_string();

    if !looks_like_an_email(&new_email) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Not a valid email address.".to_string(),
        ));
    }

    if user
        .email
        .as_deref()
        .is_some_and(|current| current.eq_ignore_ascii_case(&new_email))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "That is already this account's email.".to_string(),
        ));
    }

    // Fail before creating any pending state: a no-op sender would otherwise
    // report success for a change nobody can ever confirm.
    if !state.email_sender.is_configured() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Email is not configured on this server, so an address change cannot be verified."
                .to_string(),
        ));
    }

    if let Some(existing) =
        auth::UserRepository::get_user_by_email(&*state.data_service, &new_email)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Could not check that email.".to_string(),
                )
            })?
        && existing.id != user.id
    {
        return Err((
            StatusCode::CONFLICT,
            "That email is already in use.".to_string(),
        ));
    }

    let expires_at = Utc::now() + EMAIL_CHANGE_TOKEN_TTL;
    let request = data_service::EmailChangeWriter::create_email_change_request(
        &*state.data_service,
        user.id,
        &new_email,
        expires_at,
    )
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not start the email change.".to_string(),
        )
    })?;

    state
        .email_sender
        .send_email_change_verification(
            &new_email,
            &EmailChangeVerificationData {
                token: request.token.to_string(),
                expires_in_minutes: EMAIL_CHANGE_TOKEN_TTL.num_minutes(),
            },
        )
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, user_id = %user.id.0, "failed to send email-change verification");
            (
                StatusCode::BAD_GATEWAY,
                "Could not send the verification email. Try again.".to_string(),
            )
        })?;

    Ok(StatusCode::ACCEPTED)
}

/// Confirm a pending email change with the code sent to the new address.
#[utoipa::path(
    post,
    path = "/users/me/email/confirm",
    tag = "users",
    request_body = ConfirmEmailChangePayload,
    responses(
        (status = 204, description = "Email changed"),
        (status = 400, description = "Invalid or expired verification code"),
        (status = 409, description = "Email already in use"),
    )
)]
pub async fn confirm_email_change<A>(
    State(state): State<PgAppState<A>>,
    Json(payload): Json<ConfirmEmailChangePayload>,
) -> Result<StatusCode, (StatusCode, String)>
where
    A: SessionService + 'static,
{
    let pending = data_service::EmailChangeWriter::consume_email_change_request(
        &*state.data_service,
        payload.token,
    )
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not verify that code.".to_string(),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "That verification code is invalid or has expired.".to_string(),
        )
    })?;

    // The full `User`, not `UserInfo`: `update_user` writes the whole record,
    // including the kdf/recovery fields `UserInfo` never carries, and it must
    // see them unchanged to accept the write at all.
    let mut current = auth::UserRepository::get_user(&*state.data_service, pending.user_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not load the account.".to_string(),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            "That account no longer exists.".to_string(),
        ))?;

    current.email = Some(pending.new_email);

    auth::UserRepository::update_user(&*state.data_service, &current)
        .await
        .map_err(|e| match e {
            auth::AuthError::UserExists(_) => (
                StatusCode::CONFLICT,
                "That email is already in use.".to_string(),
            ),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not update the account.".to_string(),
            ),
        })?;

    tracing::info!(user_id = %pending.user_id.0, "account email changed after verification");
    Ok(StatusCode::NO_CONTENT)
}

/// Whether removing the email is safe, and the refusal to send back if not.
///
/// Pulled out of `remove_email` as a pure function: the handler is generic
/// over `PgAppState<A>` and has no data-free way to exercise this guard, so
/// the rule that actually matters here needs a form a test can call without a
/// database.
///
/// Refuses when the account would be left with no wallet: with neither an
/// email nor a wallet, recovery falls back to the account id, which a
/// merchant may never have saved - so removal here would strand recovery
/// rather than merely narrow it. Mirrors `WalletAuthService::revoke_wallet`'s
/// symmetric guard against removing the last wallet from an account with no
/// email (`CannotRemovePrimaryWallet` in the auth crate).
fn email_removal_blocker(has_email: bool, has_wallet: bool) -> Option<(StatusCode, String)> {
    if !has_email {
        return Some((StatusCode::BAD_REQUEST, "No email is set.".to_string()));
    }

    if !has_wallet {
        return Some((
            StatusCode::CONFLICT,
            "Removing your email would leave this account with no way to recover it. \
             Add a wallet first, or keep the email."
                .to_string(),
        ));
    }

    None
}

/// Remove the authenticated account's email.
///
/// Refused when the account has no wallet: with neither an email nor a
/// wallet, recovery falls back to the account id, which a merchant may never
/// have saved - so removal here would strand recovery rather than merely
/// narrow it. Mirrors `WalletAuthService::revoke_wallet`'s symmetric guard
/// against removing the last wallet from an account with no email
/// (`CannotRemovePrimaryWallet` in the auth crate).
#[utoipa::path(
    delete,
    path = "/users/me/email",
    tag = "users",
    security(("bearer_auth" = [])),
    responses(
        (status = 204, description = "Email removed"),
        (status = 400, description = "No email set"),
        (status = 401, description = "Unauthorized, or session is not fresh enough"),
        (status = 409, description = "Removing the email would strand recovery"),
    )
)]
pub async fn remove_email<A>(
    FreshlyAuthenticatedUser(user): FreshlyAuthenticatedUser,
    State(state): State<PgAppState<A>>,
) -> Result<StatusCode, (StatusCode, String)>
where
    A: SessionService + 'static,
{
    if let Some(err) =
        email_removal_blocker(user.email.is_some(), user.primary_wallet_address.is_some())
    {
        return Err(err);
    }

    let mut current = auth::UserRepository::get_user(&*state.data_service, user.id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not load the account.".to_string(),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            "That account no longer exists.".to_string(),
        ))?;

    current.email = None;

    auth::UserRepository::update_user(&*state.data_service, &current)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not update the account.".to_string(),
            )
        })?;

    tracing::info!(user_id = %user.id.0, "account email removed at its owner's request");
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod email_change_tests {
    use super::*;

    #[test]
    fn accepts_ordinary_addresses() {
        assert!(looks_like_an_email("merchant@example.com"));
        assert!(looks_like_an_email("  merchant@example.com  "));
    }

    #[test]
    fn rejects_obviously_wrong_shapes() {
        for bad in [
            "",
            "no-at-sign",
            "@nodomain.com",
            "nolocal@",
            "two@ats@example.com",
            "nodot@localhost",
            "trailing@dot.",
            "leading@.dot.com",
        ] {
            assert!(!looks_like_an_email(bad), "{bad:?} should not pass");
        }
    }

    #[test]
    fn removal_is_refused_with_no_wallet_to_fall_back_on() {
        assert!(matches!(
            email_removal_blocker(true, false),
            Some((StatusCode::CONFLICT, _))
        ));
    }

    #[test]
    fn removal_is_allowed_when_a_wallet_remains() {
        assert!(
            email_removal_blocker(true, true).is_none(),
            "a wallet is still a recovery handle, so removal is safe"
        );
    }

    #[test]
    fn removing_an_email_that_is_not_there_is_refused_too() {
        assert!(matches!(
            email_removal_blocker(false, true),
            Some((StatusCode::BAD_REQUEST, _))
        ));
    }
}

#[cfg(test)]
mod account_deletion_tests {
    use super::*;
    use data_service::AccountDeletionBlockers;

    fn user_with(email: Option<&str>) -> auth::UserInfo {
        auth::UserInfo {
            id: auth::UserId(uuid::Uuid::from_u128(1)),
            email: email.map(str::to_string),
            primary_wallet_address: None,
            created_at: chrono::Utc::now(),
            last_login_at: None,
            role: auth::Role::User,
        }
    }

    #[test]
    fn an_account_with_an_email_confirms_with_it() {
        assert_eq!(
            deletion_confirmation_for(&user_with(Some("merchant@example.com"))),
            "merchant@example.com"
        );
    }

    #[test]
    fn a_passkey_only_account_confirms_with_its_id() {
        // No email and no wallet: the id is the only handle it has.
        let user = user_with(None);
        assert_eq!(deletion_confirmation_for(&user), user.id.0.to_string());
    }

    #[test]
    fn confirmation_ignores_case_and_padding() {
        assert!(deletion_confirmation_matches(
            "merchant@example.com",
            "  Merchant@Example.com "
        ));
    }

    #[test]
    fn a_different_address_does_not_confirm() {
        assert!(!deletion_confirmation_matches(
            "merchant@example.com",
            "someone@example.com"
        ));
        assert!(!deletion_confirmation_matches("merchant@example.com", ""));
    }

    #[test]
    fn nothing_recorded_means_nothing_blocks() {
        assert!(!AccountDeletionBlockers::default().any());
        assert_eq!(AccountDeletionBlockers::default().describe(), "");
    }

    #[test]
    fn each_kind_of_record_blocks_on_its_own() {
        for b in [
            AccountDeletionBlockers {
                payments: 1,
                ..Default::default()
            },
            AccountDeletionBlockers {
                payouts: 1,
                ..Default::default()
            },
            AccountDeletionBlockers {
                refunds: 1,
                ..Default::default()
            },
        ] {
            assert!(b.any(), "{b:?} should block deletion");
        }
    }

    #[test]
    fn the_refusal_names_every_kind_it_found() {
        let b = AccountDeletionBlockers {
            payments: 3,
            payouts: 1,
            refunds: 2,
        };
        let msg = b.describe();
        assert!(msg.contains("3 payment(s)"), "{msg}");
        assert!(msg.contains("1 payout(s)"), "{msg}");
        assert!(msg.contains("2 refund(s)"), "{msg}");
    }
}
