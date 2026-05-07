//! Admin API endpoints.
//!
//! All endpoints require `AdminAuth` (ServerAdmin role).
//! Covers user management and server-wide settings.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use auth::{
    Role, ServerSettings, ServerSettingsRepository, SessionService, UserId, UserRepository,
};

use super::extractors::AdminAuth;
use crate::state::PgAppState;

// ============================================================================
// Types
// ============================================================================

/// Paginated user list response.
#[derive(Debug, Serialize, ToSchema)]
pub struct UserListResponse {
    pub users: Vec<AdminUserInfo>,
    pub total: i64,
    pub offset: i64,
    pub limit: i64,
}

/// User info for admin views (excludes sensitive key material).
#[derive(Debug, Serialize, ToSchema)]
pub struct AdminUserInfo {
    pub id: String,
    pub email: Option<String>,
    pub primary_wallet_address: Option<String>,
    pub role: String,
    pub created_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
    pub locked_until: Option<DateTime<Utc>>,
}

/// Query params for user listing.
#[derive(Debug, Deserialize)]
pub struct ListUsersParams {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Request body for role update.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateRoleRequest {
    pub role: String,
}

/// Server settings response (returns defaults if no row).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ServerSettingsResponse {
    pub default_confirmations: i32,
    pub invoice_expiry_minutes: i32,
    pub rate_limit_rpm: i32,
    pub enabled_chain_ids: Vec<i64>,
}

/// Request body for updating server settings.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateServerSettingsRequest {
    pub default_confirmations: i32,
    pub invoice_expiry_minutes: i32,
    pub rate_limit_rpm: i32,
    pub enabled_chain_ids: Vec<i64>,
}

// ============================================================================
// Handlers
// ============================================================================

/// List all users (paginated).
#[utoipa::path(
    get,
    path = "/admin/users",
    tag = "admin",
    security(("bearer_auth" = [])),
    params(
        ("limit" = Option<i64>, Query, description = "Max results (default 50)"),
        ("offset" = Option<i64>, Query, description = "Offset for pagination"),
    ),
    responses(
        (status = 200, description = "Paginated user list", body = UserListResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin access required"),
    )
)]
pub async fn list_users<A>(
    AdminAuth(_admin): AdminAuth,
    Query(params): Query<ListUsersParams>,
    State(state): State<PgAppState<A>>,
) -> Result<Json<UserListResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let limit = params.limit.unwrap_or(50).min(200);
    let offset = params.offset.unwrap_or(0);

    let ds = &*state.data_service;

    let users = UserRepository::list_users(ds, offset, limit)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let total = UserRepository::count_users(ds)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let admin_users: Vec<AdminUserInfo> = users
        .iter()
        .map(|u| AdminUserInfo {
            id: u.id.0.to_string(),
            email: u.email.clone(),
            primary_wallet_address: u.primary_wallet_address.clone(),
            role: u.role.as_str().to_string(),
            created_at: u.created_at,
            last_login_at: u.last_login_at,
            locked_until: u.locked_until,
        })
        .collect();

    Ok(Json(UserListResponse {
        users: admin_users,
        total,
        offset,
        limit,
    }))
}

/// Change a user's role.
///
/// Guard: cannot demote the last remaining ServerAdmin.
#[utoipa::path(
    patch,
    path = "/admin/users/{id}/role",
    tag = "admin",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "User ID")),
    request_body = UpdateRoleRequest,
    responses(
        (status = 200, description = "Role updated"),
        (status = 400, description = "Invalid role or last admin"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin access required"),
        (status = 404, description = "User not found"),
    )
)]
pub async fn update_user_role<A>(
    AdminAuth(_admin): AdminAuth,
    Path(user_id): Path<String>,
    State(state): State<PgAppState<A>>,
    Json(body): Json<UpdateRoleRequest>,
) -> Result<StatusCode, (StatusCode, &'static str)>
where
    A: SessionService + 'static,
{
    let new_role: Role = body
        .role
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid role"))?;

    let uid = uuid::Uuid::parse_str(&user_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid user ID"))?;
    let uid = UserId(uid);

    let ds = &*state.data_service;

    let mut user = ds
        .get_user(uid)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
        .ok_or((StatusCode::NOT_FOUND, "User not found"))?;

    // Guard: cannot demote the last ServerAdmin
    if user.role == Role::ServerAdmin && new_role != Role::ServerAdmin {
        let all_users = UserRepository::list_users(ds, 0, 10000)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;
        let admin_count = all_users
            .iter()
            .filter(|u| u.role == Role::ServerAdmin)
            .count();
        if admin_count <= 1 {
            return Err((
                StatusCode::BAD_REQUEST,
                "Cannot demote the last server admin",
            ));
        }
    }

    user.role = new_role;
    ds.update_user(&user)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    Ok(StatusCode::OK)
}

/// Lock a user account (set locked_until to far future).
#[utoipa::path(
    post,
    path = "/admin/users/{id}/lock",
    tag = "admin",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "User ID")),
    responses(
        (status = 200, description = "User locked"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin access required"),
        (status = 404, description = "User not found"),
    )
)]
pub async fn lock_user<A>(
    AdminAuth(_admin): AdminAuth,
    Path(user_id): Path<String>,
    State(state): State<PgAppState<A>>,
) -> Result<StatusCode, (StatusCode, &'static str)>
where
    A: SessionService + 'static,
{
    let uid = uuid::Uuid::parse_str(&user_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid user ID"))?;
    let uid = UserId(uid);

    // Lock until year 9999 (effectively permanent)
    let far_future = chrono::DateTime::parse_from_rfc3339("9999-12-31T23:59:59Z")
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Date parse error"))?
        .with_timezone(&Utc);

    state
        .data_service
        .lock_user(uid, far_future)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Failed to lock user"))?;

    Ok(StatusCode::OK)
}

/// Unlock a user account.
#[utoipa::path(
    post,
    path = "/admin/users/{id}/unlock",
    tag = "admin",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "User ID")),
    responses(
        (status = 200, description = "User unlocked"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin access required"),
        (status = 404, description = "User not found"),
    )
)]
pub async fn unlock_user<A>(
    AdminAuth(_admin): AdminAuth,
    Path(user_id): Path<String>,
    State(state): State<PgAppState<A>>,
) -> Result<StatusCode, (StatusCode, &'static str)>
where
    A: SessionService + 'static,
{
    let uid = uuid::Uuid::parse_str(&user_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid user ID"))?;
    let uid = UserId(uid);

    state
        .data_service
        .unlock_user(uid)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Failed to unlock user"))?;

    Ok(StatusCode::OK)
}

/// Get server settings (returns defaults if not yet configured).
#[utoipa::path(
    get,
    path = "/admin/settings",
    tag = "admin",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Server settings", body = ServerSettingsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin access required"),
    )
)]
pub async fn get_settings<A>(
    AdminAuth(_admin): AdminAuth,
    State(state): State<PgAppState<A>>,
) -> Result<Json<ServerSettingsResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let settings = state
        .data_service
        .get_server_settings()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .unwrap_or_default();

    Ok(Json(ServerSettingsResponse {
        default_confirmations: settings.default_confirmations,
        invoice_expiry_minutes: settings.invoice_expiry_minutes,
        rate_limit_rpm: settings.rate_limit_rpm,
        enabled_chain_ids: settings.enabled_chain_ids,
    }))
}

/// Update server settings (upsert).
#[utoipa::path(
    put,
    path = "/admin/settings",
    tag = "admin",
    security(("bearer_auth" = [])),
    request_body = UpdateServerSettingsRequest,
    responses(
        (status = 200, description = "Settings updated"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin access required"),
    )
)]
pub async fn update_settings<A>(
    AdminAuth(_admin): AdminAuth,
    State(state): State<PgAppState<A>>,
    Json(body): Json<UpdateServerSettingsRequest>,
) -> Result<StatusCode, StatusCode>
where
    A: SessionService + 'static,
{
    let settings = ServerSettings {
        default_confirmations: body.default_confirmations,
        invoice_expiry_minutes: body.invoice_expiry_minutes,
        rate_limit_rpm: body.rate_limit_rpm,
        enabled_chain_ids: body.enabled_chain_ids,
    };

    state
        .data_service
        .upsert_server_settings(&settings)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_user_list_response_serialization() {
        let resp = UserListResponse {
            users: vec![AdminUserInfo {
                id: "abc-123".to_string(),
                email: Some("test@example.com".to_string()),
                primary_wallet_address: None,
                role: "server_admin".to_string(),
                created_at: Utc::now(),
                last_login_at: None,
                locked_until: None,
            }],
            total: 1,
            offset: 0,
            limit: 50,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["total"], 1);
        assert_eq!(json["users"][0]["role"], "server_admin");
    }

    #[test]
    fn test_server_settings_response_serialization() {
        let resp = ServerSettingsResponse {
            default_confirmations: 3,
            invoice_expiry_minutes: 60,
            rate_limit_rpm: 100,
            enabled_chain_ids: vec![1, 137],
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["default_confirmations"], 3);
        assert_eq!(json["enabled_chain_ids"], serde_json::json!([1, 137]));
    }
}
