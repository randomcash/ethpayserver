//! Admin API endpoints.
//!
//! All endpoints require `AdminAuth` (ServerAdmin role).
//! Covers user management and server-wide settings.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use auth::{
    Role, ServerSettings, ServerSettingsRepository, SessionService, UserId, UserRepository,
};

use super::extractors::AdminAuth;
use crate::state::PgAppState;
pub use api_types::{
    AdminUserInfo, ServerSettingsResponse, UpdateRoleRequest, UpdateServerSettingsRequest,
    UserListResponse,
};

// ============================================================================
// Types
// ============================================================================

/// Query params for user listing.
#[derive(Debug, Deserialize)]
pub struct ListUsersParams {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Whether this boot has every plugin disabled.
///
/// Not in `api-types`: the full plugin admin surface a future page will
/// build (listing plugins and disabling them individually) belongs in the
/// shared contract once it exists. This is the minimal, honest slice of it
/// that exists today - see `payserver-client`'s `src/api/types/local.rs` for
/// the same "hand-mirror until it earns a shared contract" convention.
///
/// Consumed by `payserver-client`'s `AdminTab`
/// (`src/pages/settings/admin.rs`), which fetches this endpoint and shows a
/// banner when `safe_mode` is true - that UI ships in the same ticket's
/// `payserver-client` PR, not this repository.
#[derive(Debug, Serialize, ToSchema)]
pub struct SafeModeResponse {
    /// True when `ETHPAY_DISABLE_PLUGINS`/`--disable-plugins` was set at
    /// boot. Plugins are disabled, not uninstalled - their files and data are
    /// untouched, and clearing the flag on the next boot restores them.
    pub safe_mode: bool,
}

/// One installed plugin, as an admin needs to see it.
///
/// Carries `enabled` and `running` separately because they answer different
/// questions and routinely disagree. `enabled` is what the database records
/// and what the next boot will honour; `running` is whether the host has a
/// live, non-disabled instance right now. A plugin that is enabled but not
/// running is either a safe-mode boot or one that has crashed since startup,
/// and collapsing the two into one field is how an admin ends up restarting
/// a server to fix something a restart will not fix.
#[derive(Debug, Serialize, ToSchema)]
pub struct AdminPluginInfo {
    pub id: String,
    pub version: String,
    /// What the install record says about the next boot.
    pub enabled: bool,
    /// Whether the host holds a live, enabled instance right now.
    pub running: bool,
    /// Why it is off, when the host was the one that turned it off.
    pub disabled_reason: Option<String>,
    /// Consecutive failed calls, from the host. `0` when it is not loaded.
    pub consecutive_failures: u32,
    pub installed_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

/// The installed-plugins list.
#[derive(Debug, Serialize, ToSchema)]
pub struct AdminPluginListResponse {
    pub plugins: Vec<AdminPluginInfo>,
    /// Repeated from `GET /admin/safe-mode` so the list is self-explaining:
    /// without it, every plugin reading `enabled: true, running: false` looks
    /// like a fleet of crashes rather than one flag.
    pub safe_mode: bool,
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
        (status = 422, description = "Malformed body — e.g. a chain id that is not CAIP-2"),
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

/// Whether this boot has every plugin disabled.
#[utoipa::path(
    get,
    path = "/admin/safe-mode",
    tag = "admin",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Safe mode status", body = SafeModeResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin access required"),
    )
)]
pub async fn get_safe_mode<A>(
    AdminAuth(_admin): AdminAuth,
    State(state): State<PgAppState<A>>,
) -> Json<SafeModeResponse>
where
    A: SessionService + 'static,
{
    Json(SafeModeResponse {
        safe_mode: state.safe_mode,
    })
}

/// List installed plugins, what the next boot will do with each, and what
/// the host is doing with each right now.
///
/// The database is the authority on what is installed - the host only knows
/// what it managed to load, so asking it alone would silently omit exactly
/// the plugins an admin opened this page to find.
#[utoipa::path(
    get,
    path = "/admin/plugins",
    tag = "admin",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Installed plugins", body = AdminPluginListResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin access required"),
    )
)]
pub async fn list_plugins<A>(
    AdminAuth(_admin): AdminAuth,
    State(state): State<PgAppState<A>>,
) -> Result<Json<AdminPluginListResponse>, super::ApiErr>
where
    A: SessionService + 'static,
{
    let installed =
        data_service::InstalledPluginReader::list_installed_plugins(&*state.data_service)
            .await
            .map_err(|e| {
                super::ApiErr::from((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("could not read installed plugins: {e}"),
                ))
            })?;

    let plugins = installed
        .into_iter()
        .map(|row| {
            // A row whose id no longer parses cannot be looked up in the
            // host, but it is still installed and still the admin's to
            // remove - so it is listed as not running rather than hidden.
            let snapshot = payserver_plugin_api::PluginId::new(row.id.clone())
                .ok()
                .and_then(|id| state.plugin_host.as_ref().and_then(|h| h.status(&id)));

            AdminPluginInfo {
                id: row.id,
                version: row.version,
                enabled: row.enabled,
                running: snapshot.as_ref().is_some_and(|s| s.enabled),
                // The host's live reason wins over the stored one: if a
                // plugin was disabled after this boot started, the database
                // still says why it was disabled last time, which is the
                // wrong answer to "why is it off now".
                disabled_reason: snapshot
                    .as_ref()
                    .and_then(|s| s.disabled_reason.clone())
                    .or(row.disabled_reason),
                consecutive_failures: snapshot.as_ref().map_or(0, |s| s.consecutive_failures),
                installed_at: row.installed_at,
                updated_at: row.updated_at,
            }
        })
        .collect();

    Ok(Json(AdminPluginListResponse {
        plugins,
        safe_mode: state.safe_mode,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use types::ChainId;

    /// `GET /admin/plugins` must be mounted on the router the server serves.
    ///
    /// Three pieces of this repository have shipped fully tested and reachable
    /// from nothing - `api::plugins::router()` among them, with nine passing
    /// tests and no mount - so a handler with green unit tests is not evidence
    /// that a request can reach it. This asks the production `api::router()`.
    ///
    /// Probed with POST, which the route does not accept: axum answers 405 for
    /// a path that is registered under another method and 404 for one that is
    /// not registered at all, which separates "mounted" from "missing" without
    /// a database, a session or an admin.
    #[tokio::test]
    async fn the_plugin_list_is_mounted_on_the_real_router() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://admin-route-test-unused/db")
            .expect("connect_lazy only validates the URL, it does not connect");
        let data_service = std::sync::Arc::new(data_service::PgDataService::new(pool));
        let auth_service = std::sync::Arc::new(auth::AuthService::with_config(
            std::sync::Arc::clone(&data_service),
            auth::AuthConfig::default(),
        ));
        let state = crate::state::AppState::new(
            data_service,
            auth_service,
            None,
            std::sync::Arc::new(NoRatesForRouting),
            std::sync::Arc::new(crate::services::email::NoopEmailSender),
        );
        let app = crate::api::router(state, false, None, None, None);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST") // the route is GET-only
                    .uri("/admin/plugins")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_ne!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "/admin/plugins is not mounted on the production router; an admin \
             cannot see what is installed through a handler nothing routes to"
        );
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    /// The router needs a rate provider; the probe above never reaches one,
    /// because the request is refused on method before any handler runs.
    struct NoRatesForRouting;

    #[async_trait::async_trait]
    impl rates::RateProvider for NoRatesForRouting {
        async fn get_rate(
            &self,
            _from: &str,
            _to: &str,
        ) -> Result<rates::ExchangeRate, rates::RateError> {
            unreachable!("a 405 is decided by the router, before any handler runs")
        }

        fn name(&self) -> &'static str {
            "none"
        }
    }

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
            enabled_chain_ids: vec![ChainId::evm(1), ChainId::evm(137)],
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["default_confirmations"], 3);
        assert_eq!(
            json["enabled_chain_ids"],
            serde_json::json!(["eip155:1", "eip155:137"])
        );
    }

    #[test]
    fn test_safe_mode_response_serialization() {
        let resp = SafeModeResponse { safe_mode: true };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["safe_mode"], true);
    }
}
