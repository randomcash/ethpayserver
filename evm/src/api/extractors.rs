//! Authentication extractors for EVM API endpoints.
//!
//! This module provides axum extractors for validating authentication
//! and authorization on API endpoints.

use axum::{
    extract::FromRequestParts,
    http::{StatusCode, header::AUTHORIZATION, request::Parts},
};

use auth::{Permission, SessionId, SessionService, UserInfo};

use super::{EvmDataServiceReader, EvmState};

/// Extractor that validates admin authentication.
///
/// Extracts the session ID from the `Authorization: Bearer <session_id>` header,
/// validates the session, and checks that the user has admin permissions.
///
/// # Usage
///
/// ```rust,ignore
/// async fn admin_endpoint(
///     AdminAuth(user): AdminAuth,
///     State(state): State<EvmState<R>>,
/// ) -> impl IntoResponse {
///     // user is a UserInfo with admin role
/// }
/// ```
pub struct AdminAuth(pub UserInfo);

/// Extractor that validates any authenticated user.
///
/// Similar to AdminAuth but doesn't require admin permissions.
#[allow(dead_code)]
pub struct AuthenticatedUser(pub UserInfo);

/// Extract session ID from Authorization header.
fn extract_session_id(parts: &Parts) -> Result<SessionId, (StatusCode, String)> {
    let auth_header = parts.headers.get(AUTHORIZATION).ok_or((
        StatusCode::UNAUTHORIZED,
        "Missing Authorization header".into(),
    ))?;

    let auth_str = auth_header.to_str().map_err(|_| {
        (
            StatusCode::UNAUTHORIZED,
            "Invalid Authorization header".into(),
        )
    })?;

    // Expect "Bearer <session_id>" format
    let token = auth_str.strip_prefix("Bearer ").ok_or((
        StatusCode::UNAUTHORIZED,
        "Invalid Authorization format, expected 'Bearer <token>'".into(),
    ))?;

    // Parse session ID as UUID
    let uuid = uuid::Uuid::parse_str(token)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid session ID format".into()))?;

    Ok(SessionId(uuid))
}

impl<D, A> FromRequestParts<EvmState<D, A>> for AdminAuth
where
    D: EvmDataServiceReader + 'static,
    A: SessionService + 'static,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &EvmState<D, A>,
    ) -> Result<Self, Self::Rejection> {
        // Extract session ID from header
        let session_id = extract_session_id(parts)?;

        // Validate session
        let (user_info, _session) = state
            .auth_service
            .validate_session(session_id)
            .await
            .map_err(|e| {
                (
                    StatusCode::UNAUTHORIZED,
                    format!("Session validation failed: {}", e),
                )
            })?;

        // Check admin permission
        if !user_info
            .role
            .has_permission(Permission::ServerManageTokens)
        {
            return Err((StatusCode::FORBIDDEN, "Admin access required".into()));
        }

        Ok(AdminAuth(user_info))
    }
}

impl<D, A> FromRequestParts<EvmState<D, A>> for AuthenticatedUser
where
    D: EvmDataServiceReader + 'static,
    A: SessionService + 'static,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &EvmState<D, A>,
    ) -> Result<Self, Self::Rejection> {
        // Extract session ID from header
        let session_id = extract_session_id(parts)?;

        // Validate session
        let (user_info, _session) = state
            .auth_service
            .validate_session(session_id)
            .await
            .map_err(|e| {
                (
                    StatusCode::UNAUTHORIZED,
                    format!("Session validation failed: {}", e),
                )
            })?;

        Ok(AuthenticatedUser(user_info))
    }
}
