//! Admin API endpoints.
//!
//! All endpoints require `AdminAuth` (ServerAdmin role).
//! Covers user management and server-wide settings.
//!
//! `plugins` is the plugin lifecycle: install, upgrade, enable, disable,
//! uninstall, and the audit trail of all of it. It lives in its own module
//! because it is the only part of this surface that writes to disk and
//! changes what code the server will execute on its next boot.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use ::types::ChainId;
use auth::{
    Role, ServerSettings, ServerSettingsRepository, SessionService, UserId, UserRepository,
};

pub mod plugins;

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

/// Get server settings.
///
/// When no row has ever been written, this reports the numeric defaults
/// alongside the chain ids the server is actually gating on right now - not
/// `ServerSettings::default()`'s hardcoded mainnet list, which this process
/// may not be enforcing at all. See `unconfigured_chain_ids`.
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
    let row = state
        .data_service
        .get_server_settings()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // No row means `chain_has_no_adapter` is not using `enabled_chain_ids` at
    // all - it falls back to `unconfigured_chain_ids()` instead (see there).
    // Reporting `ServerSettings::default()`'s hardcoded mainnet list here was
    // the bug: it showed chains the server was not actually gating on, and
    // saving anything else on the page wrote that list verbatim, which does
    // not contain Sepolia. Reporting the same set the server is actually
    // using makes an unrelated save a no-op instead of a change.
    let enabled_chain_ids = match &row {
        Some(settings) => settings.enabled_chain_ids.clone(),
        None => unconfigured_chain_ids(),
    };
    let settings = row.unwrap_or_default();

    Ok(Json(ServerSettingsResponse {
        default_confirmations: settings.default_confirmations,
        invoice_expiry_minutes: settings.invoice_expiry_minutes,
        rate_limit_rpm: settings.rate_limit_rpm,
        enabled_chain_ids,
        billing_store_id: settings.billing_store_id,
        // What this process resolved at boot, compared with what is stored.
        // They differ after a change nobody has restarted into, and an admin
        // needs to be able to tell - otherwise the page shows a store the
        // server is not actually billing on.
        billing_store_id_active: state.billing_store_id == settings.billing_store_id,
    }))
}

/// The chains a build accepts for a brand-new payment method when no
/// `server_settings` row exists yet, mirroring `chain_has_no_adapter`'s
/// `None` + `New` branch exactly - see that function's doc for why this is
/// `evm::testnet` and not the full registry. Its own function so `get_settings`
/// can report the same set it is actually gated by, rather than a value that
/// looks current but isn't.
fn unconfigured_chain_ids() -> Vec<ChainId> {
    evm::testnet::ALL_TESTNETS
        .iter()
        .map(|config| ChainId::evm(config.chain_id))
        .collect()
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
    AdminAuth(admin): AdminAuth,
    State(state): State<PgAppState<A>>,
    Json(body): Json<UpdateServerSettingsRequest>,
) -> Result<StatusCode, StatusCode>
where
    A: SessionService + 'static,
{
    let current = state
        .data_service
        .get_server_settings()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .unwrap_or_default();

    // Absent leaves it alone; `Some(None)` clears it. A plain `Option` could
    // not tell those apart, and every client that saves the other four
    // settings without knowing about this field would switch billing off.
    let billing_store_id = match body.billing_store_id {
        Some(next) => next,
        None => current.billing_store_id,
    };

    if let Some(store_id) = billing_store_id
        && billing_store_id != current.billing_store_id
    {
        validate_billing_store(&state, store_id).await?;
        tracing::info!(
            actor = %admin.id,
            store_id = %store_id,
            "billing store changed; it takes effect on the next restart"
        );
    }

    // Absent leaves the stored list alone. A stored `enabled_chain_ids` is
    // authoritative - every chain not in it is refused - and the GET above
    // answers with compiled-in mainnet defaults when no row exists, so a
    // client round-tripping what it was handed would write a list nobody
    // chose. On a testnet deployment that list has no Sepolia in it.
    let enabled_chain_ids = body.enabled_chain_ids.unwrap_or(current.enabled_chain_ids);

    let settings = ServerSettings {
        default_confirmations: body.default_confirmations,
        invoice_expiry_minutes: body.invoice_expiry_minutes,
        rate_limit_rpm: body.rate_limit_rpm,
        enabled_chain_ids,
        billing_store_id,
    };

    state
        .data_service
        .upsert_server_settings(&settings)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::OK)
}

/// Refuse a billing store that could not actually be billed on.
///
/// Checked here rather than at boot because here there is a human to tell.
/// Every one of these failures is silent otherwise: the setting saves, the
/// server restarts, and the first sign of trouble is a merchant clicking Pay
/// and getting nothing - by which point nobody connects it to a settings
/// change made days earlier.
///
/// Not a foreign key, for the reason the migration gives: a settings row must
/// not be what stops a store being deleted.
async fn validate_billing_store<A>(
    state: &PgAppState<A>,
    store_id: types::StoreId,
) -> Result<(), StatusCode>
where
    A: SessionService + 'static,
{
    let methods = data_service::StorePaymentMethodReader::get_enabled_payment_methods(
        &*state.data_service,
        store_id.0,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if let Err(reason) = billable(&methods) {
        tracing::warn!(%store_id, reason, "refused a billing store that cannot be invoiced on");
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }

    Ok(())
}

/// Whether an invoice could actually be issued and paid on these methods.
///
/// Its own function so the rule is testable without a database. Both
/// failures produce the same refusal but not the same log line - an operator
/// who enabled no method and one whose wallet does not resolve have
/// different things to go and fix.
fn billable(methods: &[data_service::StorePaymentMethod]) -> Result<(), &'static str> {
    if methods.is_empty() {
        return Err("the store has no enabled payment method");
    }
    // `get_enabled_payment_methods` resolves each method's wallet through the
    // pin -> store override -> account primary walk, so this reads the
    // *resolved* answer and not the raw column. A store where nothing
    // resolves can quote no address, so an invoice issued on it could never
    // be paid.
    if methods.iter().all(|m| m.wallet_id.is_none()) {
        return Err("no enabled payment method resolves to a wallet");
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use types::ChainId;

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
            billing_store_id: None,
            billing_store_id_active: true,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["default_confirmations"], 3);
        assert_eq!(
            json["enabled_chain_ids"],
            serde_json::json!(["eip155:1", "eip155:137"])
        );
    }

    #[test]
    fn test_unconfigured_chain_ids_contains_sepolia_not_mainnet_default() {
        let chain_ids = unconfigured_chain_ids();
        assert!(chain_ids.contains(&ChainId::evm(11_155_111)));
        // The bug this guards: an unconfigured deployment must never report
        // the mainnet default list, because a testnet instance actually
        // gates on this set - not that one - and saving it verbatim would
        // start refusing Sepolia.
        assert_ne!(chain_ids, ServerSettings::default().enabled_chain_ids);
    }

    #[test]
    fn test_safe_mode_response_serialization() {
        let resp = SafeModeResponse { safe_mode: true };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["safe_mode"], true);
    }

    fn method(wallet: Option<uuid::Uuid>) -> data_service::StorePaymentMethod {
        data_service::StorePaymentMethod {
            id: uuid::Uuid::new_v4(),
            store_id: uuid::Uuid::new_v4(),
            chain_id: ChainId::evm(11155111),
            token_address: None,
            asset_symbol: "USDC".to_string(),
            decimals: 6,
            wallet_id: wallet,
            xpub: wallet.map(|_| "xpub".to_string()),
            derivation_index: wallet.map(|_| 0),
            enabled: true,
            created_at: chrono::Utc::now(),
        }
    }

    /// A billing store is checked when it is set, because that is the only
    /// moment a human is present. Every one of these failures is otherwise
    /// silent until a merchant clicks Pay and gets nothing, days later.
    #[test]
    fn a_store_that_cannot_be_invoiced_on_is_refused() {
        assert_eq!(
            billable(&[]),
            Err("the store has no enabled payment method"),
            "a store with nothing enabled can quote no asset"
        );

        assert_eq!(
            billable(&[method(None)]),
            Err("no enabled payment method resolves to a wallet"),
            "a method with no resolved wallet can quote no address, so its \
             invoice could never be paid"
        );

        assert!(billable(&[method(Some(uuid::Uuid::new_v4()))]).is_ok());

        // One resolving method is enough - the invoice can be paid on that
        // one, and refusing the whole store because a second is unconfigured
        // would block a working setup.
        assert!(billable(&[method(None), method(Some(uuid::Uuid::new_v4()))]).is_ok());
    }
}
