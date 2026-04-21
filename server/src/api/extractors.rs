//! Authentication extractors for API endpoints.
//!
//! Provides axum extractors for validating authentication and authorization.
//! Supports both session-based (Bearer UUID) and API key (Bearer ak_...) auth.

use axum::{
    extract::FromRequestParts,
    http::{StatusCode, header::AUTHORIZATION, request::Parts},
};
use chrono::Utc;

use auth::{
    ApiKeyId, ApiKeyRepository, Permission, Role, SessionId, SessionService, UserId, UserInfo,
    UserRepository,
};

use crate::state::PgAppState;

/// Marker stored in request extensions when an API key is deprecated but still
/// within the grace window.  The deprecation-header middleware reads this to set
/// the `X-API-Key-Deprecated` response header.
#[derive(Debug, Clone)]
pub struct ApiKeyDeprecationDeadline(pub chrono::DateTime<Utc>);

/// Extractor that validates any authenticated user.
///
/// Supports two auth methods (tried in order):
/// 1. Session: `Authorization: Bearer <uuid>` — validated via `SessionService`
/// 2. API key: `Authorization: Bearer ak_...` — validated via SHA-256 hash lookup
///
/// For API key auth, the key's deprecation status is checked:
/// - Deprecated but within grace → request proceeds, deprecation deadline stored in extensions
/// - Deprecated past grace → rejected with 401
pub struct AuthenticatedUser(pub UserInfo);

/// Extractor that validates server admin authentication.
///
/// Same as AuthenticatedUser but requires ServerAdmin role.
pub struct AdminAuth(pub UserInfo);

/// Extract the Bearer token from the Authorization header.
fn extract_bearer_token(parts: &Parts) -> Result<String, (StatusCode, &'static str)> {
    let auth_header = parts
        .headers
        .get(AUTHORIZATION)
        .ok_or((StatusCode::UNAUTHORIZED, "Missing Authorization header"))?;

    let auth_str = auth_header
        .to_str()
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid Authorization header"))?;

    auth_str
        .strip_prefix("Bearer ")
        .map(|s| s.to_owned())
        .ok_or((StatusCode::UNAUTHORIZED, "Invalid Authorization format"))
}

/// Validate session and return user info.
async fn validate_session<A>(
    token: &str,
    state: &PgAppState<A>,
) -> Result<UserInfo, (StatusCode, &'static str)>
where
    A: SessionService + 'static,
{
    let uuid = uuid::Uuid::parse_str(token)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid session ID format"))?;

    let (user_info, _session) = state
        .auth_service
        .validate_session(SessionId(uuid))
        .await
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid or expired session"))?;

    Ok(user_info)
}

/// Hash an API key with SHA-256 (same algorithm as key creation).
fn hash_api_key(raw_key: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(raw_key.as_bytes());
    hex::encode(hasher.finalize())
}

/// Default API key deprecation grace period: 48 hours.
const DEFAULT_DEPRECATION_GRACE_SECS: i64 = 172800;

/// Read the grace period from env (or use the 48h default).
fn deprecation_grace_secs() -> i64 {
    std::env::var("API_KEY_DEPRECATION_GRACE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_DEPRECATION_GRACE_SECS)
}

/// Validate an API key and return user info.
///
/// Also checks deprecation status — if the key is deprecated but within the grace
/// window, the deprecation deadline is stored in `parts.extensions` so the
/// middleware layer can add the `X-API-Key-Deprecated` response header.
async fn validate_api_key<A>(
    token: &str,
    parts: &mut Parts,
    state: &PgAppState<A>,
) -> Result<UserInfo, (StatusCode, &'static str)>
where
    A: SessionService + 'static,
{
    let key_hash = hash_api_key(token);

    let key_info = state
        .data_service
        .get_api_key_auth_info(&key_hash)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
        .ok_or((StatusCode::UNAUTHORIZED, "Invalid API key"))?;

    if !key_info.is_active {
        return Err((StatusCode::UNAUTHORIZED, "API key revoked"));
    }

    // Check expiration
    if let Some(expires_at) = key_info.expires_at
        && Utc::now() >= expires_at
    {
        return Err((StatusCode::UNAUTHORIZED, "API key expired"));
    }

    // Check deprecation grace window
    if let Some(deprecated_at) = key_info.deprecated_at {
        let grace = chrono::Duration::seconds(deprecation_grace_secs());
        let deadline = deprecated_at + grace;

        if Utc::now() >= deadline {
            return Err((StatusCode::UNAUTHORIZED, "API key deprecated"));
        }

        // Still within grace — store deadline for the response header
        parts.extensions.insert(ApiKeyDeprecationDeadline(deadline));
    }

    // Update last_used (fire-and-forget)
    let ds = state.data_service.clone();
    let kid = ApiKeyId(key_info.id);
    tokio::spawn(async move {
        let _ = ApiKeyRepository::update_last_used(&*ds, kid).await;
    });

    // Look up the user
    let user = state
        .data_service
        .get_user(UserId(key_info.user_id))
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
        .ok_or((StatusCode::UNAUTHORIZED, "User not found"))?;

    Ok(UserInfo::from(&user))
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
        let token = extract_bearer_token(parts)?;

        // Dispatch based on token format
        let user_info = if token.starts_with("ak_") {
            validate_api_key(&token, parts, state).await?
        } else {
            validate_session(&token, state).await?
        };

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
        let token = extract_bearer_token(parts)?;

        let user_info = if token.starts_with("ak_") {
            validate_api_key(&token, parts, state).await?
        } else {
            validate_session(&token, state).await?
        };

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
