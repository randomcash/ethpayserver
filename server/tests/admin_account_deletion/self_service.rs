//! `DELETE /users/me` (`server::api::users::delete_account`), against a real
//! database.
//!
//! Lives alongside the admin-route tests because it exercises the exact
//! guard `account.rs` does: `active_watched_addresses`, shared between the
//! admin path and this one specifically so self-service deletion cannot
//! destroy a pending, unconfirmed payment any more easily than an admin
//! could. Before that guard was shared, this path had no defense against it
//! at all - see `deleting_your_own_account_with_a_still_watched_address_is_refused`.

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
async fn deleting_your_own_account_with_a_still_watched_address_is_refused() {
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
    // Never paid - the case `account_deletion_blockers` would miss, and the
    // one `active_watched_addresses` exists for.
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
    let Err((status, message)) = result else {
        panic!("an account with a still-watched address must be refused");
    };
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        message.contains("watch"),
        "the caller must see why, got: {message}"
    );

    let no_command_arrived = tokio::time::timeout(Duration::from_millis(500), commands.next())
        .await
        .is_err();
    assert!(
        no_command_arrived,
        "a refused delete must not unwatch the account's still-live invoice"
    );

    let still_there: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id = $1")
        .bind(target)
        .fetch_one(state.data_service.pool())
        .await
        .expect("count target");
    assert_eq!(still_there, 1, "the refusal must not have deleted anything");

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
