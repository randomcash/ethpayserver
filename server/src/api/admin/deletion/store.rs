//! The synthetic-E2E-only store hard-delete path.

use axum::{
    extract::{Path, State},
    http::StatusCode,
};

use auth::{SessionService, StoreId, UserId, repository::StoreRepository};
use data_service::{PayoutReader, RefundReader};

use super::{active_watched_addresses, unwatch_after_delete};
use crate::api::extractors::AdminAuth;
use crate::state::PgAppState;

/// The exact name shape the synthetic-payment E2E job gives the stores it
/// creates (`e2e/tests/synthetic-payment.spec.ts`,
/// `new Date().toISOString().replace(/[:.]/g, '-')`). Mirrors
/// `SYNTHETIC_STORE_NAME` in `e2e/scripts/sweep-e2e-stores.mjs` - kept in
/// sync by hand since one side is Rust and the other JavaScript.
///
/// A name match alone is not proof of where a store came from - a store's
/// name is ordinary caller-supplied input, so anyone who can create a store
/// can give it this exact shape. `hard_delete_store` also requires the store
/// to be owned by [`E2E_STORE_OWNER_ID`], which is what actually keeps this
/// endpoint from reaching a real merchant's store.
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

/// The only account `synthetic-payment.spec.ts` ever creates stores under -
/// mirrors `PROTECTED_USER_IDS` in `e2e/scripts/sweep-e2e-accounts.mjs`, kept
/// in sync by hand for the same reason [`is_synthetic_e2e_store_name`]
/// mirrors that script's `SYNTHETIC_STORE_NAME` regex.
///
/// `hard_delete_store` requires a store to match this *and* the name shape
/// before it will remove it. Either check alone is spoofable by an ordinary
/// caller (a name is just a string; ownership by itself says nothing about
/// what a store is named) - together they mean a real merchant's store,
/// however it happens to be named, is never both.
pub const E2E_STORE_OWNER_ID: &str = "c5ae10f2-da34-4002-af3b-5a4ec8b6ec97";

fn is_e2e_store_owner(owner_id: UserId) -> bool {
    owner_id.0.to_string() == E2E_STORE_OWNER_ID
}

/// Refuse with a 409 if `sid` holds any payout or refund.
///
/// Called by `hard_delete_store` immediately before the delete itself, for
/// the same reason `ensure_no_financial_blockers` is.
async fn ensure_no_payout_or_refund(
    ds: &data_service::PgDataService,
    sid: StoreId,
) -> Result<(), (StatusCode, String)> {
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

    Ok(())
}

/// payment methods.
///
/// This is not `DELETE /stores/{id}`: that endpoint archives, on purpose, so
/// a merchant's invoices and payments stay readable for a post-mortem after
/// the store leaves their store list. This endpoint actually removes the
/// rows, which is why it is gated by name *and* ownership rather than by
/// financial history:
///
/// - The name must match the exact `e2e-synthetic-<ISO timestamp>` shape
///   `synthetic-payment.spec.ts` gives the store it creates on every
///   scheduled run - unlike `delete_user_account`, this endpoint cannot lean
///   on "no financial history" to stay off a real merchant's store, because
///   removing a paid synthetic invoice is the entire point.
/// - The store must be owned by [`E2E_STORE_OWNER_ID`]. A name is ordinary
///   caller-supplied input - nothing stops a real merchant, or anyone probing
///   the pattern, from naming a store the same way - so the name check alone
///   is not proof of where a store came from. Ownership is: `E2E_STORE_OWNER_ID`
///   is the one account `synthetic-payment.spec.ts` ever creates stores
///   under, and neither check alone is enough to keep this endpoint off a
///   real merchant's store.
/// - Payouts and refunds against the store are checked anyway and block the
///   delete: `ON DELETE CASCADE` does not cover them (see
///   `data_service::account_deletion` for why), so a raw delete would either
///   silently destroy that history or fail on the foreign key. Neither the
///   synthetic-payment job nor the backfill sweep should ever produce one,
///   so seeing one here means something matched that should not have.
#[utoipa::path(
    delete,
    path = "/admin/stores/{id}",
    tag = "admin",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Store ID")),
    responses(
        (status = 204, description = "Store deleted"),
        (status = 400, description = "Invalid store ID, or the store does not match the synthetic E2E name and owner"),
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

    if !is_synthetic_e2e_store_name(&store.name) || !is_e2e_store_owner(store.owner_id) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Only a store named like, and owned by, the synthetic-payment \
             E2E job's own account can be hard-deleted through this endpoint."
                .to_string(),
        ));
    }

    // Read now, unwatched later: the cascade below removes these rows, and
    // by the time it has run there is nothing left in Postgres to read them
    // from.
    let addresses = active_watched_addresses(&state, std::slice::from_ref(&sid.0)).await?;

    ensure_no_payout_or_refund(ds, sid).await?;

    StoreRepository::delete_store(ds, sid).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not delete the store.".to_string(),
        )
    })?;

    // Only now, with the store actually gone - see `unwatch_after_delete`
    // for why this cannot run any earlier.
    unwatch_after_delete(state.evm_monitor.as_deref(), addresses).await;

    tracing::info!(actor = %admin.id, store_id = %sid, "synthetic E2E store hard-deleted by admin");
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    /// This regex is half of `hard_delete_store`'s safety property - it can
    /// reach a real merchant's store the moment this accepts something it
    /// should not, though `is_e2e_store_owner` still has to agree too.
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

    /// The other half of `hard_delete_store`'s safety property: a name match
    /// by itself is a string comparison against caller-supplied input, so it
    /// proves nothing about who actually created the store.
    #[test]
    fn e2e_store_owner_matches_only_the_known_account() {
        let owner: uuid::Uuid = E2E_STORE_OWNER_ID.parse().expect("valid uuid literal");
        assert!(is_e2e_store_owner(UserId(owner)));
        assert!(!is_e2e_store_owner(UserId(uuid::Uuid::new_v4())));
    }
}
