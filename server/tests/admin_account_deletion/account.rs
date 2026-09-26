//! `DELETE /admin/users/{id}` and `GET /admin/users/{id}/stores`, against a
//! real database.
//!
//! Both handlers wrap logic that is already covered elsewhere - the financial
//! blockers in `data-service/src/postgres/integration_tests/account_deletion.rs`,
//! the cascade in the same file, `AdminAuth`'s admin-only gate in every other
//! admin route - but nothing exercised the two checks that live only in
//! `delete_user_account` itself: refusing a `server_admin` target outright,
//! and turning a blocked deletion into the 409 an operator (or an automated
//! sweep) actually sees. An automated sweep against this endpoint is exactly
//! what widens its blast radius if either check silently stops firing.

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use evm::monitor::{COMMANDS_CHANNEL, EVENTS_CHANNEL, EventBridge, RedisBridge};
use tokio_stream::StreamExt;
use uuid::Uuid;

use auth::{Store, UserId};
use data_service::store_creation::StoreCreationWriter;
use server::api::admin::{delete_user_account, list_user_stores};
use server::services::RedisEVMMonitor;

use crate::support::*;

/// The guard the ticket's automated sweep depends on: whatever matches a
/// cleanup query must not be able to reach the one account a deployment
/// cannot lose just because it also matched.
#[tokio::test]
#[ignore]
async fn deleting_a_server_admin_target_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let caller = seed_user(pg.pool(), "server_admin").await;
    let target = seed_user(pg.pool(), "server_admin").await;
    let state = app_state(Arc::new(pg));

    let result = delete_user_account(
        admin_auth(caller),
        Path(target.to_string()),
        State(state.clone()),
    )
    .await;

    let Err((status, _)) = result else {
        panic!("deleting a server_admin target must be refused");
    };
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let still_there: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id = $1")
        .bind(target)
        .fetch_one(state.data_service.pool())
        .await
        .expect("count target");
    assert_eq!(still_there, 1, "the refusal must not have deleted anything");

    cleanup(state.data_service.pool(), &[caller, target]).await;
}

/// The same guard self-service `DELETE /users/me` relies on, reached through
/// the admin path: an admin sweeping abandoned accounts must not be able to
/// destroy a merchant's payment history any more easily than the merchant
/// could destroy their own.
#[tokio::test]
#[ignore]
async fn deleting_an_account_that_took_a_payment_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let caller = seed_user(pg.pool(), "server_admin").await;
    let target = seed_user(pg.pool(), "user").await;
    let store = Store::new(format!("store-{target}"), UserId(target));
    pg.create_store_owned_by(&store, UserId(target))
        .await
        .expect("seed store owned by target");
    let invoice = seed_invoice(pg.pool(), store.id.0).await;
    seed_payment(pg.pool(), &invoice).await;

    let state = app_state(Arc::new(pg));

    let result = delete_user_account(
        admin_auth(caller),
        Path(target.to_string()),
        State(state.clone()),
    )
    .await;

    let Err((status, message)) = result else {
        panic!("an account that took a payment must be refused");
    };
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        message.contains("payment"),
        "the operator must see why, got: {message}"
    );

    let still_there: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id = $1")
        .bind(target)
        .fetch_one(state.data_service.pool())
        .await
        .expect("count target");
    assert_eq!(still_there, 1, "the refusal must not have deleted anything");

    cleanup(state.data_service.pool(), &[caller, target]).await;
}

/// The success path an automated sweep actually depends on: an account with
/// nothing blocking it is removed, and its store goes with it through the
/// same cascade self-service deletion uses.
#[tokio::test]
#[ignore]
async fn deleting_an_untraded_account_succeeds_and_takes_its_store() {
    let Some(pg) = service().await else {
        return;
    };
    let caller = seed_user(pg.pool(), "server_admin").await;
    let target = seed_user(pg.pool(), "user").await;
    let store = Store::new(format!("store-{target}"), UserId(target));
    pg.create_store_owned_by(&store, UserId(target))
        .await
        .expect("seed store owned by target");

    let state = app_state(Arc::new(pg));

    let result = delete_user_account(
        admin_auth(caller),
        Path(target.to_string()),
        State(state.clone()),
    )
    .await;

    assert_eq!(result, Ok(StatusCode::NO_CONTENT));

    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id = $1")
        .bind(target)
        .fetch_one(state.data_service.pool())
        .await
        .expect("count users");
    assert_eq!(users, 0);

    let stores: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stores WHERE id = $1")
        .bind(store.id.0)
        .fetch_one(state.data_service.pool())
        .await
        .expect("count stores");
    assert_eq!(stores, 0, "the store should have gone with the account");

    cleanup(state.data_service.pool(), &[caller]).await;
}

/// What an admin sees before deciding whether an account is safe to remove
/// must be scoped to that account, not leak another user's stores into the
/// answer.
#[tokio::test]
#[ignore]
async fn list_user_stores_is_scoped_to_the_requested_user() {
    let Some(pg) = service().await else {
        return;
    };
    let caller = seed_user(pg.pool(), "server_admin").await;
    let target = seed_user(pg.pool(), "user").await;
    let other = seed_user(pg.pool(), "user").await;
    let target_store = Store::new(format!("mine-{target}"), UserId(target));
    let other_store = Store::new(format!("theirs-{other}"), UserId(other));
    pg.create_store_owned_by(&target_store, UserId(target))
        .await
        .expect("seed target store");
    pg.create_store_owned_by(&other_store, UserId(other))
        .await
        .expect("seed other store");

    let state = app_state(Arc::new(pg));

    let Ok(Json(stores)) = list_user_stores(
        admin_auth(caller),
        Path(target.to_string()),
        State(state.clone()),
    )
    .await
    else {
        panic!("listing an existing user's stores must succeed");
    };

    assert_eq!(stores.len(), 1, "must not see another user's stores");
    assert_eq!(stores[0].name, target_store.name);

    cleanup(state.data_service.pool(), &[caller, target, other]).await;
}

/// A pending, still-watched invoice may have a real payment broadcast to it
/// that just hasn't confirmed yet - `account_deletion_blockers` cannot see
/// this because nothing about it is *recorded*. Deleting the account anyway
/// and unwatching afterwards (which an earlier version of this handler did)
/// would tell the monitor to stop looking right as the invoice it was
/// watching for is cascaded away, permanently losing the ability to credit
/// that payment. This must be refused instead - and, like
/// `hard_delete_store_refused_by_a_payout_never_tells_the_monitor_to_unwatch`,
/// a real monitor listening on the commands channel is the only way to prove
/// the still-live address was never touched, not just that the account
/// survives.
#[tokio::test]
#[ignore]
async fn deleting_an_account_with_a_still_watched_address_is_refused() {
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

    let caller = seed_user(pg.pool(), "server_admin").await;
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

    let result = delete_user_account(
        admin_auth(caller),
        Path(target.to_string()),
        State(state.clone()),
    )
    .await;
    let Err((status, message)) = result else {
        panic!("an account with a still-watched address must be refused");
    };
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        message.contains("watch"),
        "the operator must see why, got: {message}"
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

    cleanup(state.data_service.pool(), &[caller, target]).await;
}
