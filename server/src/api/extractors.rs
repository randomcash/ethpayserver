//! Authentication extractors for API endpoints.
//!
//! Provides axum extractors for validating authentication and authorization.
//! Supports both session tokens (UUID) and API keys (ak_XXXX_YYYY).

use axum::{
    extract::FromRequestParts,
    http::{StatusCode, header::AUTHORIZATION, request::Parts},
};

use auth::{Permission, Role, SessionId, SessionService, UserId, UserInfo};
use chrono::{DateTime, Utc};

use super::api_key_deprecation::DeprecationSlot;
use super::api_key_hash::hash_api_key;
use crate::state::PgAppState;

/// Carried in the shared `DeprecationSlot` that the deprecation-header
/// middleware installs into request extensions before the handler runs.
/// `validate_api_key` writes into the slot when an authenticated request
/// uses a deprecated-but-still-valid API key, and the middleware reads it
/// afterwards to stamp the `X-API-Key-Deprecated` response header.
///
/// Request extensions set from inside a handler/extractor are NOT visible
/// to the outer middleware after `next.run(req)` — that's why the transport
/// is an Arc-shared slot rather than a direct extension write.
#[derive(Debug, Clone)]
pub struct ApiKeyDeprecationInfo {
    pub deprecated_at: DateTime<Utc>,
    pub grace_deadline: DateTime<Utc>,
}

/// Extractor that validates any authenticated user.
///
/// Supports both session tokens and API keys:
/// - `Authorization: Bearer <uuid>` → session-based auth
/// - `Authorization: Bearer ak_...` → API key auth
pub struct AuthenticatedUser(pub UserInfo);

/// Extractor that validates server admin authentication.
///
/// Same as AuthenticatedUser but requires ServerAdmin role.
pub struct AdminAuth(pub UserInfo);

/// Extract the bearer token string from the Authorization header.
fn extract_bearer_token(parts: &Parts) -> Result<String, (StatusCode, &'static str)> {
    let auth_header = parts
        .headers
        .get(AUTHORIZATION)
        .ok_or((StatusCode::UNAUTHORIZED, "Missing Authorization header"))?;

    let auth_str = auth_header
        .to_str()
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid Authorization header"))?;

    let token = auth_str
        .strip_prefix("Bearer ")
        .ok_or((StatusCode::UNAUTHORIZED, "Invalid Authorization format"))?;

    Ok(token.to_string())
}

/// Validate session and return user info.
///
/// Takes `&mut Parts` so the API-key branch can stamp `ApiKeyDeprecationInfo`
/// into request extensions when a deprecated-but-still-valid key is used.
async fn validate_session<A>(
    parts: &mut Parts,
    state: &PgAppState<A>,
) -> Result<UserInfo, (StatusCode, &'static str)>
where
    A: SessionService + 'static,
{
    let token = extract_bearer_token(parts)?;

    // If the token starts with "ak_", validate as API key
    if token.starts_with("ak_") {
        return validate_api_key(&token, parts, state).await;
    }

    // Otherwise treat as session UUID
    let uuid = uuid::Uuid::parse_str(&token)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid session ID format"))?;

    let session_id = SessionId(uuid);

    let (user_info, _session) = state
        .auth_service
        .validate_session(session_id)
        .await
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid or expired session"))?;

    Ok(user_info)
}

/// Validate an API key and return the associated user info.
///
/// When the key is deprecated but within its grace window, stamps an
/// `ApiKeyDeprecationInfo` into `parts.extensions` so the response-header
/// middleware can emit `X-API-Key-Deprecated: rotate before <iso>`.
async fn validate_api_key<A>(
    raw_key: &str,
    parts: &mut Parts,
    state: &PgAppState<A>,
) -> Result<UserInfo, (StatusCode, &'static str)>
where
    A: SessionService + 'static,
{
    let key_hash = hash_api_key(raw_key);

    let key_info = state
        .data_service
        .get_api_key_auth_info(&key_hash)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
        .ok_or((StatusCode::UNAUTHORIZED, "Invalid API key"))?;

    // Check if active
    if !key_info.is_active {
        return Err((StatusCode::UNAUTHORIZED, "API key revoked"));
    }

    // Check expiration
    if key_info
        .expires_at
        .is_some_and(|expires_at| Utc::now() > expires_at)
    {
        return Err((StatusCode::UNAUTHORIZED, "API key expired"));
    }

    // Check deprecation grace window
    if let Some(deprecated_at) = key_info.deprecated_at {
        let grace_secs = deprecation_grace_secs();
        let now = Utc::now();
        if is_grace_expired(deprecated_at, now, grace_secs) {
            return Err((
                StatusCode::UNAUTHORIZED,
                "API key deprecated and grace period expired",
            ));
        }
        let deadline = deprecated_at + chrono::Duration::seconds(grace_secs);
        // Write into the shared slot the deprecation-header middleware
        // installed. Direct `parts.extensions.insert` would be invisible to
        // the outer middleware after `next.run` — only an Arc-shared handle
        // survives that boundary. If the middleware isn't wired (tests,
        // unusual router builds) we quietly skip: auth still succeeds, the
        // consumer just doesn't see the header on this one request.
        if let Some(slot) = parts.extensions.get::<DeprecationSlot>()
            && let Ok(mut guard) = slot.lock()
        {
            *guard = Some(ApiKeyDeprecationInfo {
                deprecated_at,
                grace_deadline: deadline,
            });
        }
    }

    // Resolve the user via data_service (PgDataService implements UserRepository)
    use auth::UserRepository;
    let user = state
        .data_service
        .get_user(UserId(key_info.user_id))
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Failed to resolve user"))?
        .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "User not found"))?;
    let user = UserInfo::from(&user);

    // Fire-and-forget: update last_used_at
    let ds = state.data_service.clone();
    let key_id = key_info.id;
    tokio::spawn(async move {
        let _ = sqlx::query("UPDATE api_keys SET last_used_at = NOW() WHERE id = $1")
            .bind(key_id)
            .execute(ds.pool())
            .await;
    });

    Ok(user)
}

/// Get the deprecation grace period in seconds (default: 48 hours).
///
/// Also used by `users::list_api_keys` to compute the
/// `deprecation_expires_at` deadline surfaced on list responses, so the
/// client can show an accurate expiry even when operators override the
/// grace window via `API_KEY_DEPRECATION_GRACE_SECS`.
pub(super) fn deprecation_grace_secs() -> i64 {
    std::env::var("API_KEY_DEPRECATION_GRACE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(172_800) // 48 hours
}

impl<A> FromRequestParts<PgAppState<A>> for AuthenticatedUser
where
    A: SessionService + 'static,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &PgAppState<A>,
    ) -> Result<Self, Self::Rejection> {
        let user_info = validate_session(parts, state).await?;
        Ok(AuthenticatedUser(user_info))
    }
}

impl<A> FromRequestParts<PgAppState<A>> for AdminAuth
where
    A: SessionService + 'static,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &PgAppState<A>,
    ) -> Result<Self, Self::Rejection> {
        let user_info = validate_session(parts, state).await?;

        // Check for ServerAdmin role
        if user_info.role != Role::ServerAdmin {
            return Err((StatusCode::FORBIDDEN, "Admin access required"));
        }

        Ok(AdminAuth(user_info))
    }
}

impl AuthenticatedUser {
    /// Get the user ID.
    pub fn user_id(&self) -> UserId {
        self.0.id
    }

    /// Get the user's role.
    pub fn role(&self) -> Role {
        self.0.role
    }

    /// Check if user has a specific permission.
    pub fn has_permission(&self, permission: Permission) -> bool {
        self.0.role.has_permission(permission)
    }
}

impl AdminAuth {
    /// Get the user ID.
    pub fn user_id(&self) -> UserId {
        self.0.id
    }
}

/// Pure predicate: is a deprecated key past its grace window at `now`?
///
/// Extracted so the grace-expiry rule can be unit-tested without booting a
/// database or touching the extractor wiring. Matches the live check in
/// `validate_api_key` exactly.
pub(super) fn is_grace_expired(
    deprecated_at: DateTime<Utc>,
    now: DateTime<Utc>,
    grace_secs: i64,
) -> bool {
    now > deprecated_at + chrono::Duration::seconds(grace_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn at(hour: i64) -> DateTime<Utc> {
        // Fixed base date so tests are deterministic; chrono's Utc::now drift
        // would otherwise race the grace-window arithmetic.
        Utc.with_ymd_and_hms(2026, 4, 23, 0, 0, 0).unwrap() + Duration::hours(hour)
    }

    const GRACE_48H: i64 = 48 * 3600;

    #[test]
    fn in_grace_is_not_expired() {
        // deprecated at hour 0, now at hour 24, 48h grace → still valid
        assert!(!is_grace_expired(at(0), at(24), GRACE_48H));
    }

    #[test]
    fn exactly_at_deadline_is_not_expired() {
        // at the exact boundary we're still inside; strictly > means at == not expired
        assert!(!is_grace_expired(at(0), at(48), GRACE_48H));
    }

    #[test]
    fn past_deadline_is_expired() {
        // 1 second past the 48h grace
        let deadline = at(0) + Duration::hours(48);
        assert!(is_grace_expired(
            at(0),
            deadline + Duration::seconds(1),
            GRACE_48H
        ));
    }

    #[test]
    fn zero_grace_means_immediate_expiry_next_moment() {
        assert!(!is_grace_expired(at(0), at(0), 0));
        assert!(is_grace_expired(at(0), at(0) + Duration::seconds(1), 0));
    }

    #[test]
    fn long_grace_keeps_key_valid() {
        // 30-day grace
        let grace = 30 * 24 * 3600;
        assert!(!is_grace_expired(at(0), at(24 * 20), grace));
        assert!(is_grace_expired(at(0), at(24 * 31), grace));
    }
}
