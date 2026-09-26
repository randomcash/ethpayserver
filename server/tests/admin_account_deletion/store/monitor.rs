//! The two `DELETE /admin/stores/{id}` tests that need a live
//! `RedisEVMMonitor`, proving the unwatch step actually fires (or doesn't,
//! when the delete is refused) rather than merely that the delete succeeds.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use evm::monitor::{COMMANDS_CHANNEL, EVENTS_CHANNEL, EventBridge, MonitorCommand, RedisBridge};
use tokio_stream::StreamExt;
use uuid::Uuid;

use auth::{Store, UserId};
use data_service::store_creation::StoreCreationWriter;
use server::api::admin::hard_delete_store;
use server::services::RedisEVMMonitor;

use crate::support::*;

/// Every test above builds `PgAppState` with `evm_monitor: None`, so
/// `unwatch_after_delete` always takes its early return - the branch that
/// actually talks to the monitor has never run in CI, and a regression that
/// broke, reordered or dropped that call would pass every test here. This is
/// the one test that wires in a real `RedisEVMMonitor` and listens on the
/// commands channel it publishes to, so it proves the delete actually tells
/// the monitor to stop watching, not merely that the delete itself succeeds.
#[tokio::test]
#[ignore]
async fn hard_delete_store_tells_a_live_monitor_to_unwatch_a_pending_invoices_address() {
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
    let target = seed_e2e_owner(pg.pool()).await;
    let store = Store::new(
        "e2e-synthetic-2026-01-04T00-00-00-000Z".to_string(),
        UserId(target),
    );
    pg.create_store_owned_by(&store, UserId(target))
        .await
        .expect("seed store");
    let invoice = seed_invoice(pg.pool(), store.id.0).await;
    // Never paid - the case `get_active_watched_addresses_for_stores` exists
    // for, and the one every other query in this file's blocker checks would
    // miss.
    let address = format!("0x{:040x}", Uuid::new_v4().as_u128());
    let payment_option = seed_payment_option(pg.pool(), &invoice, &address).await;
    seed_watched_address(pg.pool(), &invoice, payment_option, &address).await;

    let state = app_state_with_monitor(Arc::new(pg), Some(Arc::new(monitor)));

    let result = hard_delete_store(
        admin_auth(caller),
        Path(store.id.0.to_string()),
        State(state.clone()),
    )
    .await;
    assert_eq!(result, Ok(StatusCode::NO_CONTENT));

    let published = tokio::time::timeout(Duration::from_secs(5), commands.next())
        .await
        .expect("an unwatch command should have been published once the delete completed")
        .expect("the commands stream ended unexpectedly");

    match published {
        MonitorCommand::UnwatchAddress(cmd) => {
            assert_eq!(cmd.chain_id, 11155111);
            let expected: evm::Address = address.parse().expect("valid test address");
            assert_eq!(cmd.address, expected);
            assert_eq!(cmd.token_contract, None);
        }
        other => panic!("expected an UnwatchAddress command, got {other:?}"),
    }

    cleanup(state.data_service.pool(), &[caller, target]).await;
}

/// The ordering `unwatch_after_delete` depends on for its whole safety
/// argument: nothing gets unwatched unless the delete it follows actually
/// went through. A store with a payout is refused before the delete runs
/// (`hard_delete_store_refuses_when_a_payout_exists` covers that refusal),
/// but that test never wires in a monitor, so it cannot tell an old,
/// unwatch-before-delete ordering apart from this one - both would return the
/// same 409. This test can: with a real monitor listening, an
/// unwatch-before-delete implementation would still publish the command for
/// the store's still-pending, still-watched invoice even though the store
/// survives the refusal, silently leaving a live invoice unwatched. Nothing
/// should arrive on the channel at all.
#[tokio::test]
#[ignore]
async fn hard_delete_store_refused_by_a_payout_never_tells_the_monitor_to_unwatch() {
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
    let target = seed_e2e_owner(pg.pool()).await;
    let store = Store::new(
        "e2e-synthetic-2026-01-05T00-00-00-000Z".to_string(),
        UserId(target),
    );
    pg.create_store_owned_by(&store, UserId(target))
        .await
        .expect("seed store");
    let invoice = seed_invoice(pg.pool(), store.id.0).await;
    let address = format!("0x{:040x}", Uuid::new_v4().as_u128());
    let payment_option = seed_payment_option(pg.pool(), &invoice, &address).await;
    seed_watched_address(pg.pool(), &invoice, payment_option, &address).await;
    seed_payout(pg.pool(), store.id.0).await;

    let state = app_state_with_monitor(Arc::new(pg), Some(Arc::new(monitor)));

    let result = hard_delete_store(
        admin_auth(caller),
        Path(store.id.0.to_string()),
        State(state.clone()),
    )
    .await;
    let Err((status, _)) = result else {
        panic!("a store holding a payout must be refused");
    };
    assert_eq!(status, StatusCode::CONFLICT);

    let no_command_arrived = tokio::time::timeout(Duration::from_millis(500), commands.next())
        .await
        .is_err();
    assert!(
        no_command_arrived,
        "a refused delete must not unwatch the store's still-live invoice"
    );

    let still_there: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stores WHERE id = $1")
        .bind(store.id.0)
        .fetch_one(state.data_service.pool())
        .await
        .expect("count store");
    assert_eq!(still_there, 1, "the refusal must not have deleted anything");

    clear_payouts_for_store(state.data_service.pool(), store.id.0).await;
    cleanup(state.data_service.pool(), &[caller, target]).await;
}
