//! Listing a user's stores, and deleting the account.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use auth::{Role, SessionService, UserId, UserRepository, repository::StoreRepository};
use data_service::AccountDeletionReader;

use super::{active_watched_addresses, unwatch_after_delete};
use crate::api::extractors::AdminAuth;
use crate::api::stores::store_response;
use crate::state::PgAppState;

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

/// Refuse with a 409 if `uid`'s stores hold any payment, payout or refund.
///
/// Called by `delete_user_account` immediately before the delete itself, not
/// earlier - the only work between this check and the delete is two local DB
/// reads, so there is no unbounded window (a network round-trip to the
/// monitor, formerly done here) for the state this sees to go stale before
/// the delete runs.
async fn ensure_no_financial_blockers(
    ds: &data_service::PgDataService,
    uid: UserId,
) -> Result<(), (StatusCode, String)> {
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

    Ok(())
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
///
/// Also refuses while any of the account's stores still has an actively
/// watched address (see [`active_watched_addresses`]) - a `pending`,
/// never-expired invoice whose customer may have already broadcast a
/// transaction that has not confirmed yet. `account_deletion_blockers` only
/// sees *recorded* payments, so it passes that case untouched; unwatching
/// after the fact (as `hard_delete_store` does deliberately, for its own
/// synthetic store) would tell the monitor to stop looking right as the
/// invoice it was watching for is cascaded away, permanently unlinking a real
/// payment from any merchant credit. An account this refuses on is not
/// abandoned - a genuinely abandoned signup has no stores and so no watched
/// addresses either.
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
        (status = 409, description = "Account holds financial records, or a still-watched address, and cannot be deleted"),
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

    let owned_stores = StoreRepository::get_stores_owned_by(ds, uid)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?;
    let store_ids: Vec<uuid::Uuid> = owned_stores.iter().map(|s| s.id.0).collect();

    // Read now, unwatched later: the cascade below removes these rows, and
    // by the time it has run there is nothing left in Postgres to read them
    // from.
    let addresses = active_watched_addresses(&state, &store_ids).await?;

    if !addresses.is_empty() {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "This account has {} still-watched address(es) for a pending invoice. \
                 A payment broadcast to one of them may not have confirmed yet, and \
                 deleting now would stop watching it with nothing left to credit it to. \
                 Refused until the invoice resolves (paid, cancelled or expired).",
                addresses.len()
            ),
        ));
    }

    ensure_no_financial_blockers(ds, uid).await?;

    UserRepository::delete_user(ds, uid).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not delete the account.".to_string(),
        )
    })?;

    // Only now, with the account actually gone - see `unwatch_after_delete`
    // for why this cannot run any earlier.
    unwatch_after_delete(&state, addresses).await;

    tracing::info!(actor = %admin.id, user_id = %uid, "account deleted by admin");
    Ok(StatusCode::NO_CONTENT)
}
