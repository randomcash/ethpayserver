//! Authentication extractors for API endpoints.
//!
//! Provides axum extractors for validating authentication and authorization.
//! Supports both session tokens (UUID) and API keys (ak_XXXX_YYYY).

use axum::{
    extract::FromRequestParts,
    http::{StatusCode, header::AUTHORIZATION, request::Parts},
};

use auth::{Permission, Role, SessionId, SessionService, UserId, UserInfo};
use chrono::Utc;

use crate::state::PgAppState;

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
async fn validate_session<A>(
    parts: &Parts,
    state: &PgAppState<A>,
) -> Result<UserInfo, (StatusCode, &'static str)>
where
    A: SessionService + 'static,
{
    let token = extract_bearer_token(parts)?;

    // If the token starts with "ak_", validate as API key
    if token.starts_with("ak_") {
        return validate_api_key(&token, state).await;
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
async fn validate_api_key<A>(
    raw_key: &str,
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
        let deadline = deprecated_at + chrono::Duration::seconds(grace_secs);
        if Utc::now() > deadline {
            return Err((
                StatusCode::UNAUTHORIZED,
                "API key deprecated and grace period expired",
            ));
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

/// Hash an API key with SHA-256.
fn hash_api_key(raw_key: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(raw_key.as_bytes());
    hex::encode(hasher.finalize())
}

/// Get the deprecation grace period in seconds (default: 48 hours).
fn deprecation_grace_secs() -> i64 {
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
