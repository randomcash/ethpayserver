//! `DELETE /users/me` (`server::api::users::delete_account`), against a real
//! database.
//!
//! Lives alongside the admin-route tests because the two paths share
//! `active_watched_addresses` and `unwatch_after_delete`.
//!
//! The two paths no longer behave the same, deliberately. This one no longer
//! REFUSES on a still-watched address: the operator ruled that an unpaid
//! invoice must not stop a merchant deleting their own account, and the
//! refusal was broader than its stated purpose, which is a payment already
//! broadcast. That case is still refused, by `account_deletion_blockers`,
//! which counts any `payments` row - and detection writes one with
//! `confirmed_at = NULL`. The watches are cleared after the delete commits
//! instead of being made impossible beforehand.
//!
//! The admin path KEEPS its refusal. The ruling was about merchants deleting
//! their own accounts, and widening it is a separate decision nobody has
//! taken. So an admin is currently refused where a merchant is not - recorded
//! here as a choice rather than left to read as an oversight.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use evm::monitor::{COMMANDS_CHANNEL, EVENTS_CHANNEL, EventBridge, RedisBridge};
use tokio_stream::StreamExt;
use uuid::Uuid;

use auth::{Store, UserId};
use data_service::store_creation::StoreCreationWriter;
use server::api::users::{DeleteAccountQuery, delete_account};
use server::services::RedisEVMMonitor;

use crate::support::*;

/// The case this test suite exists for: a pending invoice's address may have
/// a real payment broadcast to it that just hasn't confirmed. Before this
/// guard was shared with the admin path, `delete_account` only checked
/// `account_deletion_blockers` - which sees *recorded* payments only - so a
/// merchant could delete their own account out from under a payment in
/// flight, leaving the monitor watching an address for an invoice that no
/// longer exists. A real monitor listening on the commands channel is the
/// only way to prove the still-live address was never touched, not just that
/// the account survives.
#[tokio::test]
#[ignore]
async fn deleting_your_own_account_with_an_unpaid_invoice_succeeds_and_unwatches() {
    let Some(pg) = service().await else {
        return;
    };
    let Some(redis_url) = std::env::var("TEST_REDIS_URL").ok() else {
        return;
    };
    let monitor = RedisEVMMonitor::connect(&redis_url)
        .await
        .unwrap_or_else(|e| panic!("TEST_REDIS_URL is set but connecting failed: {e}"));

    let subscriber = RedisBridge::new(&redis_url, EVENTS_CHANNEL, COMMANDS_CHANNEL)
        .await
        .expect("connect a second bridge to observe published commands");
    let mut commands = subscriber
        .subscribe_commands()
        .await
        .expect("subscribe to the commands channel");

    let target = seed_user(pg.pool(), "user").await;
    let store = Store::new(format!("store-{target}"), UserId(target));
    pg.create_store_owned_by(&store, UserId(target))
        .await
        .expect("seed store owned by target");
    let invoice = seed_invoice(pg.pool(), store.id.0).await;
    // Never paid. This used to be refused outright, which is what the
    // operator ruled against: an unpaid invoice must not stop a merchant
    // deleting their own account. What must still be refused is a payment
    // already DETECTED, and `account_deletion_blockers` covers that on its own
    // - detection writes a `payments` row with `confirmed_at = NULL` and that
    // count has no `confirmed_at` filter.
    let address = format!("0x{:040x}", Uuid::new_v4().as_u128());
    let payment_option = seed_payment_option(pg.pool(), &invoice, &address).await;
    seed_watched_address(pg.pool(), &invoice, payment_option, &address).await;

    let state = app_state_with_monitor(Arc::new(pg), Some(Arc::new(monitor)));

    let result = delete_account(
        self_auth(target),
        State(state.clone()),
        Query(DeleteAccountQuery {
            confirm: target.to_string(),
        }),
    )
    .await;
    let status = result.unwrap_or_else(|(status, message)| {
        panic!("an unpaid invoice must not block deletion, got {status}: {message}")
    });
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The replacement invariant, and the reason this assertion matters more
    // than the one it replaces. The old refusal made orphaned watches
    // impossible; nothing now does, except this unwatch running after the
    // delete commits. So assert the command actually goes out - a delete that
    // succeeded while leaving the monitor polling a deleted invoice is the
    // failure this change could have introduced.
    let command_arrived = tokio::time::timeout(Duration::from_millis(2_000), commands.next())
        .await
        .is_ok();
    assert!(
        command_arrived,
        "a successful delete must unwatch the addresses it just orphaned"
    );

    let gone: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id = $1")
        .bind(target)
        .fetch_one(state.data_service.pool())
        .await
        .expect("count target");
    assert_eq!(gone, 0, "the account must actually be gone");

    cleanup(state.data_service.pool(), &[target]).await;
}

/// The success path this guard must not block: an account with no watched
/// addresses at all (an abandoned signup, or a store that never got an
/// invoice) still deletes cleanly through the same cascade as before.
#[tokio::test]
#[ignore]
async fn deleting_your_own_untraded_account_still_succeeds() {
    let Some(pg) = service().await else {
        return;
    };
    let target = seed_user(pg.pool(), "user").await;
    let state = app_state(Arc::new(pg));

    let result = delete_account(
        self_auth(target),
        State(state.clone()),
        Query(DeleteAccountQuery {
            confirm: target.to_string(),
        }),
    )
    .await;

    assert_eq!(result, Ok(StatusCode::NO_CONTENT));

    let still_there: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id = $1")
        .bind(target)
        .fetch_one(state.data_service.pool())
        .await
        .expect("count target");
    assert_eq!(still_there, 0);
}
