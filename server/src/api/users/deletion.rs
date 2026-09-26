//! Self-service account deletion (`DELETE /users/me`).

use axum::{extract::State, http::StatusCode};

use auth::SessionService;

use crate::api::extractors::AuthenticatedUser;
use crate::services::plugins::notify_account_closed;
use crate::state::PgAppState;

/// Confirmation the caller must type back before the account is deleted.
///
/// A query parameter rather than a JSON body, deliberately: a shared body type
/// would have to live in `api-types` and be mirrored by the client, and a DTO
/// the two sides define separately is the drift this codebase already paid for
/// once. There is nothing secret in it - it is the account's own email or id,
/// both of which the caller must already be authenticated to know.
#[derive(Debug, serde::Deserialize)]
pub struct DeleteAccountQuery {
    /// Must equal the account's email, or its id when there is no email.
    pub confirm: String,
}

/// What the caller must type to confirm.
///
/// Email where the account has one, because it is the handle a merchant knows.
/// A passkey-only account has no email and no wallet, so its id is the only
/// thing it can be asked for - the same reason recovery accepts a UUID.
fn deletion_confirmation_for(user: &auth::UserInfo) -> String {
    user.email.clone().unwrap_or_else(|| user.id.0.to_string())
}

/// Whether what was typed matches, ignoring case and surrounding whitespace.
///
/// Case-insensitive because email is, and a merchant retyping their own address
/// with a capital letter has still proved intent. Not a secret comparison, so
/// there is nothing to keep constant-time.
fn deletion_confirmation_matches(expected: &str, typed: &str) -> bool {
    typed.trim().eq_ignore_ascii_case(expected.trim())
}

/// Delete the authenticated account.
///
/// Refuses while the account's stores hold any payment, payout or refund. That
/// is not squeamishness: `users` cascades through `stores` into `invoices` and
/// `payments`, so deleting a merchant who traded would erase the records they
/// need to answer a customer, a chargeback or a tax question - and `payouts`
/// and `refunds` are `NO ACTION`, so the same delete would fail on a foreign
/// key and surface as a 500. `DELETE /stores/{id}` already archives rather than
/// deletes for this reason; this endpoint declines rather than pretending.
///
/// Also refuses while any owned store still has an actively watched address -
/// the same guard `admin::deletion::delete_user_account` applies, for the same
/// reason: `account_deletion_blockers` only sees *recorded* payments, so a
/// `pending`, never-expired invoice whose customer has already broadcast a
/// transaction that has not confirmed yet passes it untouched. Without this, a
/// merchant could delete their own account out from under a payment in
/// flight - the cascade removes the invoice while the monitor is still
/// watching for it, and the payment that lands afterward has nothing left to
/// credit.
///
/// What it is for is the case deletion is actually asked for: an abandoned
/// signup, a test account, a merchant who never traded. Those cascade cleanly -
/// devices, sessions, passkeys, api keys, wallets, empty stores - and leave
/// nothing behind.
#[utoipa::path(
    delete,
    path = "/users/me",
    params(("confirm" = String, Query, description = "The account's email, or its id when it has no email")),
    responses(
        (status = 204, description = "Account deleted"),
        (status = 400, description = "Confirmation did not match"),
        (status = 409, description = "Account holds financial records, or a still-watched address, and cannot be deleted"),
    ),
    tag = "users"
)]
pub async fn delete_account<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    axum::extract::Query(query): axum::extract::Query<DeleteAccountQuery>,
) -> Result<StatusCode, (StatusCode, String)>
where
    A: SessionService + 'static,
{
    let expected = deletion_confirmation_for(&user);
    if !deletion_confirmation_matches(&expected, &query.confirm) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Confirmation did not match this account.".to_string(),
        ));
    }

    let owned_stores =
        auth::repository::StoreRepository::get_stores_owned_by(&*state.data_service, user.id)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Database error".to_string(),
                )
            })?;
    let store_ids: Vec<uuid::Uuid> = owned_stores.iter().map(|s| s.id.0).collect();

    // Read, but no longer refuse on. These addresses are cleared after the
    // delete commits instead - see the call at the end of this function.
    //
    // This used to return 409 whenever the list was non-empty, which blocked a
    // merchant from deleting their own account while ANY unpaid invoice lived.
    // That is broader than the reason for it: the case worth refusing is a
    // payment already broadcast, and `account_deletion_blockers` below already
    // catches that, because detection writes a `payments` row with
    // `confirmed_at = NULL` and that count has no `confirmed_at` filter. The
    // old refusal also told the merchant to cancel the invoice, which does not
    // help - cancelling deactivates the payment options and leaves
    // `watched_addresses.is_active` true.
    let addresses =
        crate::api::admin::deletion::active_watched_addresses(&state, &store_ids).await?;

    let blockers = data_service::AccountDeletionReader::account_deletion_blockers(
        &*state.data_service,
        user.id,
    )
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not check the account.".to_string(),
        )
    })?;

    if blockers.any() {
        // Named, not a bare refusal: a merchant who cannot delete needs to know
        // what is holding it, and an operator triaging this needs the same.
        return Err((
            StatusCode::CONFLICT,
            format!(
                "This account's stores hold {}. Deleting it would destroy that \
                 history, so it is refused. Archive the stores instead.",
                blockers.describe()
            ),
        ));
    }

    auth::UserRepository::delete_user(&*state.data_service, user.id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not delete the account.".to_string(),
            )
        })?;

    // Only now, with the account actually gone. This cannot run any earlier:
    // unwatching is an external side effect with no rollback, so doing it
    // before a delete that then failed would stop watching a still-live
    // invoice. Run after a commit, the worst case is the opposite and much
    // cheaper - the monitor polls a deleted invoice a little longer.
    //
    // This replaces the refusal that used to stand above. That refusal made
    // orphaned watches impossible; this makes them unlikely, and the counter
    // inside is how we learn which. The general fix is a reconciler against
    // the monitor's watch set, because Postgres cannot see a stale watch at
    // all: `watched_addresses` cascades from both `invoices` and
    // `payment_options`, so the rows are gone and the monitor's are not.
    crate::api::admin::deletion::unwatch_after_delete(&state, addresses).await;

    // After the account is actually gone, not before: a plugin holding data
    // for it must never be told "closed" for an account that a later failure
    // in this handler left alive.
    notify_account_closed(&state.account_closed_observers, user.id).await;

    tracing::info!(user_id = %user.id.0, "account deleted at its owner's request");
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use data_service::AccountDeletionBlockers;

    fn user_with(email: Option<&str>) -> auth::UserInfo {
        auth::UserInfo {
            id: auth::UserId(uuid::Uuid::from_u128(1)),
            email: email.map(str::to_string),
            primary_wallet_address: None,
            created_at: chrono::Utc::now(),
            last_login_at: None,
            role: auth::Role::User,
        }
    }

    #[test]
    fn an_account_with_an_email_confirms_with_it() {
        assert_eq!(
            deletion_confirmation_for(&user_with(Some("merchant@example.com"))),
            "merchant@example.com"
        );
    }

    #[test]
    fn a_passkey_only_account_confirms_with_its_id() {
        // No email and no wallet: the id is the only handle it has.
        let user = user_with(None);
        assert_eq!(deletion_confirmation_for(&user), user.id.0.to_string());
    }

    #[test]
    fn confirmation_ignores_case_and_padding() {
        assert!(deletion_confirmation_matches(
            "merchant@example.com",
            "  Merchant@Example.com "
        ));
    }

    #[test]
    fn a_different_address_does_not_confirm() {
        assert!(!deletion_confirmation_matches(
            "merchant@example.com",
            "someone@example.com"
        ));
        assert!(!deletion_confirmation_matches("merchant@example.com", ""));
    }

    #[test]
    fn nothing_recorded_means_nothing_blocks() {
        assert!(!AccountDeletionBlockers::default().any());
        assert_eq!(AccountDeletionBlockers::default().describe(), "");
    }

    #[test]
    fn each_kind_of_record_blocks_on_its_own() {
        for b in [
            AccountDeletionBlockers {
                payments: 1,
                ..Default::default()
            },
            AccountDeletionBlockers {
                payouts: 1,
                ..Default::default()
            },
            AccountDeletionBlockers {
                refunds: 1,
                ..Default::default()
            },
        ] {
            assert!(b.any(), "{b:?} should block deletion");
        }
    }

    #[test]
    fn the_refusal_names_every_kind_it_found() {
        let b = AccountDeletionBlockers {
            payments: 3,
            payouts: 1,
            refunds: 2,
        };
        let msg = b.describe();
        assert!(msg.contains("3 payment(s)"), "{msg}");
        assert!(msg.contains("1 payout(s)"), "{msg}");
        assert!(msg.contains("2 refund(s)"), "{msg}");
    }
}
