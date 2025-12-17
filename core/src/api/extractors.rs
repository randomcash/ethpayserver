//! Authentication extractors for API endpoints.
//!
//! Provides axum extractors for validating authentication and authorization.

use axum::{
    extract::FromRequestParts,
    http::{header::AUTHORIZATION, request::Parts, StatusCode},
};

use auth::{AuthRepository, Permission, Role, SessionId, UserInfo, UserId};

use crate::state::AppState;

/// Extractor that validates any authenticated user.
///
/// Extracts the session ID from the `Authorization: Bearer <session_id>` header
/// and validates the session.
///
/// # Usage
///
/// ```rust,ignore
/// async fn protected_endpoint(
///     AuthenticatedUser(user): AuthenticatedUser,
///     State(state): State<AppState<R>>,
/// ) -> impl IntoResponse {
///     // user is a UserInfo
/// }
/// ```
pub struct AuthenticatedUser(pub UserInfo);

/// Extractor that validates server admin authentication.
///
/// Same as AuthenticatedUser but requires ServerAdmin role.
pub struct AdminAuth(pub UserInfo);

/// Extract session ID from Authorization header.
fn extract_session_id(parts: &Parts) -> Result<SessionId, (StatusCode, &'static str)> {
    let auth_header = parts
        .headers
        .get(AUTHORIZATION)
        .ok_or((StatusCode::UNAUTHORIZED, "Missing Authorization header"))?;

    let auth_str = auth_header
        .to_str()
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid Authorization header"))?;

    // Expect "Bearer <session_id>" format
    let token = auth_str
        .strip_prefix("Bearer ")
        .ok_or((StatusCode::UNAUTHORIZED, "Invalid Authorization format"))?;

    // Parse session ID as UUID
    let uuid = uuid::Uuid::parse_str(token)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid session ID format"))?;

    Ok(SessionId(uuid))
}

impl<R> FromRequestParts<AppState<R>> for AuthenticatedUser
where
    R: AuthRepository + Send + Sync + 'static,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState<R>,
    ) -> Result<Self, Self::Rejection> {
        let session_id = extract_session_id(parts)?;

        let (user_info, _session) = state
            .auth_service
            .validate_session(session_id)
            .await
            .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid or expired session"))?;

        Ok(AuthenticatedUser(user_info))
    }
}

impl<R> FromRequestParts<AppState<R>> for AdminAuth
where
    R: AuthRepository + Send + Sync + 'static,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState<R>,
    ) -> Result<Self, Self::Rejection> {
        let session_id = extract_session_id(parts)?;

        let (user_info, _session) = state
            .auth_service
            .validate_session(session_id)
            .await
            .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid or expired session"))?;

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
