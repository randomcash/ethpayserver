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
use evm::Address;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use auth::{
    Role, ServerSettings, ServerSettingsRepository, SessionService, StoreId, UserId,
    UserRepository, repository::StoreRepository,
};
use data_service::{AccountDeletionReader, PayoutReader, RefundReader, WatchedAddressWriter};

pub mod plugins;

use super::extractors::AdminAuth;
use super::stores::store_response;
use crate::services::evm_monitor::EVMMonitor;
use crate::state::PgAppState;
pub use api_types::{
    AdminUserInfo, ServerSettingsResponse, UpdateRoleRequest, UpdateServerSettingsRequest,
    UserListResponse,
};

/// The exact name shape the synthetic-payment E2E job gives the stores it
/// creates (`e2e/tests/synthetic-payment.spec.ts`,
/// `new Date().toISOString().replace(/[:.]/g, '-')`). Mirrors
/// `SYNTHETIC_STORE_NAME` in `e2e/scripts/sweep-e2e-stores.mjs` - kept in
/// sync by hand since one side is Rust and the other JavaScript.
///
/// This is the only thing standing between `hard_delete_store` and a real
/// merchant's store: the endpoint hard-deletes on a live server, so unlike
/// `delete_user_account` it cannot lean on "no financial history" as its
/// safety property - the whole point is to remove stores that *do* have
/// recorded payments. Scoping it to a name only this one CI job ever
/// generates is what takes the place of that check.
fn is_synthetic_e2e_store_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("e2e-synthetic-") else {
        return false;
    };
    // 2026-08-27T17-29-33-596Z
    let bytes = rest.as_bytes();
    if bytes.len() != 24 {
        return false;
    }
    let digit = |i: usize| bytes[i].is_ascii_digit();
    let literal = |i: usize, c: u8| bytes[i] == c;
    (0..4).all(digit)
        && literal(4, b'-')
        && (5..7).all(digit)
        && literal(7, b'-')
        && (8..10).all(digit)
        && literal(10, b'T')
        && (11..13).all(digit)
        && literal(13, b'-')
        && (14..16).all(digit)
        && literal(16, b'-')
        && (17..19).all(digit)
        && literal(19, b'-')
        && (20..23).all(digit)
        && literal(23, b'Z')
}

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

/// List the stores a user owns.
///
/// `GET /stores` only ever answers for the caller, so an admin deciding
/// whether an account is safe to remove has no way to see what it owns.
/// This is that lookup, scoped to an arbitrary user id rather than the
/// session.
#[utoipa::path(
    get,
    path = "/admin/users/{id}/stores",
    tag = "admin",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "User ID")),
    responses(
        (status = 200, description = "Stores owned by this user", body = Vec<api_types::StoreResponse>),
        (status = 400, description = "Invalid user ID"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin access required"),
    )
)]
pub async fn list_user_stores<A>(
    AdminAuth(_admin): AdminAuth,
    Path(user_id): Path<String>,
    State(state): State<PgAppState<A>>,
) -> Result<Json<Vec<api_types::StoreResponse>>, (StatusCode, &'static str)>
where
    A: SessionService + 'static,
{
    let uid = uuid::Uuid::parse_str(&user_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid user ID"))?;
    let uid = UserId(uid);

    let stores = StoreRepository::get_stores_for_user(&*state.data_service, uid)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    Ok(Json(stores.into_iter().map(store_response).collect()))
}

/// Delete a user account.
///
/// The same safeguard as self-service `DELETE /users/me`
/// (`data_service::AccountDeletionReader`): refused while the account's
/// stores hold any payment, payout or refund, so an admin cannot destroy a
/// merchant's financial history any more easily than the merchant could.
///
/// Also refuses outright on a `ServerAdmin` target, rather than counting how
/// many remain (contrast `update_user_role`'s last-admin guard) - deleting an
/// admin is a heavier action than demoting one, and an operator who means it
/// can still demote first. This is also what stands between an automated
/// cleanup script and the one account a deployment cannot lose: whatever
/// swept a batch of abandoned signups must not be able to reach the account
/// that holds a production signing key just because it matched the same
/// query.
#[utoipa::path(
    delete,
    path = "/admin/users/{id}",
    tag = "admin",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "User ID")),
    responses(
        (status = 204, description = "Account deleted"),
        (status = 400, description = "Invalid user ID, or target is a server admin"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin access required"),
        (status = 404, description = "User not found"),
        (status = 409, description = "Account holds financial records and cannot be deleted"),
    )
)]
pub async fn delete_user_account<A>(
    AdminAuth(admin): AdminAuth,
    Path(user_id): Path<String>,
    State(state): State<PgAppState<A>>,
) -> Result<StatusCode, (StatusCode, String)>
where
    A: SessionService + 'static,
{
    let uid = uuid::Uuid::parse_str(&user_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid user ID".to_string()))?;
    let uid = UserId(uid);

    let ds = &*state.data_service;

    let user = ds
        .get_user(uid)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?
        .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    if user.role == Role::ServerAdmin {
        return Err((
            StatusCode::BAD_REQUEST,
            "Cannot delete a server admin. Demote the account first.".to_string(),
        ));
    }

    let blockers = AccountDeletionReader::account_deletion_blockers(ds, uid)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not check the account.".to_string(),
            )
        })?;

    if blockers.any() {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "This account's stores hold {}. Deleting it would destroy that \
                 history, so it is refused.",
                blockers.describe()
            ),
        ));
    }

    let owned_stores = StoreRepository::get_stores_owned_by(ds, uid)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?;
    let store_ids: Vec<uuid::Uuid> = owned_stores.iter().map(|s| s.id.0).collect();
    unwatch_stores_before_delete(&state, &store_ids).await?;

    UserRepository::delete_user(ds, uid).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not delete the account.".to_string(),
        )
    })?;

    tracing::info!(actor = %admin.id, user_id = %uid, "account deleted by admin");
    Ok(StatusCode::NO_CONTENT)
}

/// Unwatch every still-active address for invoices under `store_ids`, before
/// their stores are removed.
///
/// `account_deletion_blockers` (and, for `hard_delete_store`, the
/// payout/refund check below) only count *recorded* financial history - an
/// invoice with an address generated and watched, but no payment recorded
/// yet, passes both untouched. Deleting straight through it would leave a
/// no-TTL Redis key pointing at an invoice id that no longer exists; if a
/// payment then lands on that address, the monitor resolves the stale id and
/// the payment can never be recorded against it. Done here, before the
/// delete, rather than left to the background cleanup service, which only
/// ever unwatches expired, paid or cancelled invoices - never a still-pending
/// one, which is exactly this case.
async fn unwatch_stores_before_delete<A>(
    state: &PgAppState<A>,
    store_ids: &[uuid::Uuid],
) -> Result<(), (StatusCode, String)>
where
    A: SessionService + 'static,
{
    let Some(monitor) = &state.evm_monitor else {
        // No live monitor wired into this process, so nothing was ever
        // watched through it and there is nothing to unwatch.
        return Ok(());
    };

    let addresses = state
        .data_service
        .get_active_watched_addresses_for_stores(store_ids)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not look up watched addresses.".to_string(),
            )
        })?;

    for info in addresses {
        let addr: Address = info.address.parse().map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Watched address {} is not a valid address.", info.address),
            )
        })?;
        let token_contract: Option<Address> =
            info.token_address.as_deref().and_then(|t| t.parse().ok());
        let eip155 = info.chain_id.evm_chain_id().ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("{} is not an EVM chain.", info.chain_id),
            )
        })?;

        monitor
            .unwatch_address_by_chain_id(eip155, addr, token_contract)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Could not unwatch {}: {e}", info.address),
                )
            })?;

        WatchedAddressWriter::deactivate(
            &*state.data_service,
            &info.address,
            &info.chain_id,
            info.token_address.as_deref(),
        )
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not deactivate a watched address.".to_string(),
            )
        })?;
    }

    Ok(())
}

/// Hard-delete a store, cascading to its invoices, payments, wallets and
/// payment methods.
///
/// This is not `DELETE /stores/{id}`: that endpoint archives, on purpose, so
/// a merchant's invoices and payments stay readable for a post-mortem after
/// the store leaves their store list. This endpoint actually removes the
/// rows, which is why it is gated by name rather than by ownership or
/// financial history:
///
/// - The name must match the exact `e2e-synthetic-<ISO timestamp>` shape
///   `synthetic-payment.spec.ts` gives the store it creates on every
///   scheduled run. Nothing else can ever be named this by construction, so
///   this is what keeps the endpoint from ever reaching a real merchant's
///   store - unlike `delete_user_account`, it cannot lean on "no financial
///   history" for that, because removing a paid synthetic invoice is the
///   entire point.
/// - Payouts and refunds against the store are checked anyway and block the
///   delete: `ON DELETE CASCADE` does not cover them (see
///   `data_service::account_deletion` for why), so a raw delete would either
///   silently destroy that history or fail on the foreign key. Neither the
///   synthetic-payment job nor the backfill sweep should ever produce one,
///   so seeing one here means the name matched something it should not have.
#[utoipa::path(
    delete,
    path = "/admin/stores/{id}",
    tag = "admin",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Store ID")),
    responses(
        (status = 204, description = "Store deleted"),
        (status = 400, description = "Invalid store ID, or the name does not match the synthetic E2E pattern"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin access required"),
        (status = 404, description = "Store not found"),
        (status = 409, description = "Store holds a payout or refund and cannot be deleted"),
    )
)]
pub async fn hard_delete_store<A>(
    AdminAuth(admin): AdminAuth,
    Path(store_id): Path<String>,
    State(state): State<PgAppState<A>>,
) -> Result<StatusCode, (StatusCode, String)>
where
    A: SessionService + 'static,
{
    let sid = uuid::Uuid::parse_str(&store_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid store ID".to_string()))?;
    let sid = StoreId(sid);

    let ds = &*state.data_service;

    let store = ds
        .get_store(sid)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?
        .ok_or((StatusCode::NOT_FOUND, "Store not found".to_string()))?;

    if !is_synthetic_e2e_store_name(&store.name) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Only a store named like the synthetic-payment E2E job's own \
             stores can be hard-deleted through this endpoint."
                .to_string(),
        ));
    }

    let (payout_count, _) = PayoutReader::get_payouts_for_store(ds, sid, 1, 0)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not check payouts.".to_string(),
            )
        })?;
    let (refund_count, _) = RefundReader::get_refunds_for_store(ds, sid, 1, 0)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not check refunds.".to_string(),
            )
        })?;
    if payout_count > 0 || refund_count > 0 {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "This store holds {payout_count} payout(s) and {refund_count} refund(s), \
                 which a synthetic E2E store should never have. Refusing rather than \
                 destroying or orphaning them.",
            ),
        ));
    }

    unwatch_stores_before_delete(&state, std::slice::from_ref(&sid.0)).await?;

    StoreRepository::delete_store(ds, sid).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not delete the store.".to_string(),
        )
    })?;

    tracing::info!(actor = %admin.id, store_id = %sid, "synthetic E2E store hard-deleted by admin");
    Ok(StatusCode::NO_CONTENT)
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
        billing_store_id: settings.billing_store_id,
        // What this process resolved at boot, compared with what is stored.
        // They differ after a change nobody has restarted into, and an admin
        // needs to be able to tell - otherwise the page shows a store the
        // server is not actually billing on.
        billing_store_id_active: state.billing_store_id == settings.billing_store_id,
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

    let settings = ServerSettings {
        default_confirmations: body.default_confirmations,
        invoice_expiry_minutes: body.invoice_expiry_minutes,
        rate_limit_rpm: body.rate_limit_rpm,
        enabled_chain_ids: body.enabled_chain_ids,
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

    /// This regex is the entire safety property of `hard_delete_store` - it
    /// can reach a real merchant's store the moment this accepts something it
    /// should not.
    #[test]
    fn synthetic_e2e_store_name_matches_only_the_exact_shape() {
        assert!(is_synthetic_e2e_store_name(
            "e2e-synthetic-2026-08-27T17-29-33-596Z"
        ));

        // A human-named store that merely starts the same way.
        assert!(!is_synthetic_e2e_store_name("e2e-synthetic-scratch"));
        // Prefix only, no timestamp at all.
        assert!(!is_synthetic_e2e_store_name("e2e-synthetic-"));
        // A real merchant's store.
        assert!(!is_synthetic_e2e_store_name("My Coffee Shop"));
        // Close but wrong separators, wrong lengths, or trailing garbage.
        assert!(!is_synthetic_e2e_store_name(
            "e2e-synthetic-2026-08-27T17:29:33.596Z"
        ));
        assert!(!is_synthetic_e2e_store_name(
            "e2e-synthetic-2026-08-27T17-29-33-596Zx"
        ));
        assert!(!is_synthetic_e2e_store_name(
            "e2e-synthetic-2026-08-27T17-29-33-59Z"
        ));
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
