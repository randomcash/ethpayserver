//! User API endpoints — API key management.
//!
//! All endpoints require authentication via session token.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use auth::{ApiKey, ApiKeyId, ApiKeyInfo, ApiKeyRepository, Role, SessionService};
use data_service::ApiKeyFullInfo;

use super::api_key_hash::hash_api_key;
use super::extractors::AuthenticatedUser;
use crate::state::PgAppState;
pub use api_types::{
    ApiKeyInfoResponse, ApiKeyListResponse, CreateApiKeyPayload, CreateApiKeyResponsePayload,
    RotateApiKeyResponsePayload, UpdateApiKeyPayload,
};

/// Build from an `ApiKey` plus the ancillary rate-limit / deprecation fields
/// not present on the auth-crate struct. Used by endpoints that already
/// have an `ApiKey` in hand (e.g. update_api_key after a mutation).
pub(crate) fn api_key_info_with_rate_limit(
    key: &ApiKey,
    rate_limit_rpm: Option<i32>,
    deprecated_at: Option<DateTime<Utc>>,
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
    }
}

/// Build the wire shape from the `auth` domain type.
///
/// A free function rather than a `From` impl: `ApiKeyFullInfo` belongs to `auth` and
/// `ApiKeyInfoResponse` to `api-types`, so neither is local here. `api-types` does not
/// depend on `auth` deliberately - it is compiled into the browser bundle and
/// `auth` is a server-side crate.
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

    Ok(Json(api_key_info_with_rate_limit(
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

    // Create the replacement key + deprecate the old one atomically.
    // Without a transaction a partial failure (new key created, deprecation
    // fails) would leave TWO active keys on the account — the explicit
    // enemy of rotation.
    let new_name = format!("{} (rotated)", key.name);
    let (raw_key, new_api_key) = build_api_key(&new_name, user.id, key.expires_at);
    let now = Utc::now();

    state
        .data_service
        .rotate_api_key_atomic(&new_api_key, id, now)
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
