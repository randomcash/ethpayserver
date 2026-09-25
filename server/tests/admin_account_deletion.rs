#![allow(clippy::unwrap_used, clippy::expect_used)]

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

use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use evm::monitor::{COMMANDS_CHANNEL, EVENTS_CHANNEL, EventBridge, MonitorCommand, RedisBridge};
use sqlx::PgPool;
use tokio_stream::StreamExt;
use uuid::Uuid;

use auth::{
    Result as AuthResult, Role, Session, SessionId, SessionService, Store, UserId, UserInfo,
};
use data_service::PgDataService;
use data_service::store_creation::StoreCreationWriter;
use rates::NoOpRateProvider;
use server::api::AdminAuth;
use server::api::admin::{delete_user_account, hard_delete_store, list_user_stores};
use server::services::RedisEVMMonitor;
use server::state::PgAppState;

/// Not exercised: neither handler calls back into session management, only
/// reads the already-authenticated `AdminAuth` this test constructs directly.
struct UnusedSessionService;

#[async_trait]
impl SessionService for UnusedSessionService {
    async fn validate_session(&self, _session_id: SessionId) -> AuthResult<(UserInfo, Session)> {
        unimplemented!("not exercised by these handlers")
    }
    async fn logout(&self, _session_id: SessionId) -> AuthResult<()> {
        unimplemented!("not exercised by these handlers")
    }
    async fn logout_all(&self, _session_id: SessionId) -> AuthResult<()> {
        unimplemented!("not exercised by these handlers")
    }
    async fn cleanup_stale_sessions(&self) -> AuthResult<u64> {
        unimplemented!("not exercised by these handlers")
    }
}

async fn service() -> Option<PgDataService> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    // `?` here would collapse "not configured" and "configured but
    // unreachable" into the same skip, and a skipped test reports the same
    // green result as a passing one. These four tests are the only
    // verification that the server-admin refusal, the financial-history
    // refusal and the delete cascade actually hold - a DB that is set but
    // briefly unreachable must fail loudly, not silently report success.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .unwrap_or_else(|e| panic!("DATABASE_URL is set but connecting failed: {e}"));
    Some(PgDataService::new(pool))
}

/// `plugin_invoice_creation_filter.rs`'s `seed_user` uses `'{}'::jsonb` for
/// both blobs and gets away with it because nothing there ever reads a row
/// back through `row_to_user`. This test does - `delete_user_account` calls
/// `get_user` - and `row_to_user` deserializes both into `KdfParams` and
/// `EncryptedBlob`, so an empty object fails with "missing field `algorithm`"
/// before the handler under test ever runs.
async fn seed_user(pool: &PgPool, role: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, kdf_params, encrypted_symmetric_key, \
         recovery_verification_hash, kdf_salt_identifier, role) \
         VALUES ($1, \
           '{\"algorithm\":\"argon2id\",\"memory_kb\":65536,\"iterations\":3,\"parallelism\":4,\"salt\":\"\"}'::jsonb, \
           '{\"ciphertext\":\"\",\"iv\":\"\",\"mac\":\"\"}'::jsonb, \
           'h', 'passkey:' || $1::text, $2)",
    )
    .bind(id)
    .bind(role)
    .execute(pool)
    .await
    .expect("seed user");
    id
}

async fn seed_invoice(pool: &PgPool, store: Uuid) -> String {
    let id = format!("inv-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO invoices (id, store_id, currency, amount, expires_at) \
         VALUES ($1, $2, 'USD', 10, now() + interval '1 hour')",
    )
    .bind(&id)
    .bind(store)
    .execute(pool)
    .await
    .expect("seed invoice");
    id
}

async fn seed_payment(pool: &PgPool, invoice: &str) {
    sqlx::query(
        "INSERT INTO payments (invoice_id, chain_id, asset_type, asset_symbol, amount, tx_hash) \
         VALUES ($1, 'eip155:11155111', 'native', 'ETH', 1, $2)",
    )
    .bind(invoice)
    .bind(format!("0x{}", Uuid::new_v4().simple()))
    .execute(pool)
    .await
    .expect("seed payment");
}

/// A pending invoice's still-watched address - the case `get_active_watched_addresses_for_stores`
/// exists for, since it has no payment and so trips none of the other
/// cleanup queries (all scoped to expired/paid/cancelled invoices).
async fn seed_payment_option(pool: &PgPool, invoice: &str, address: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO payment_options \
         (id, invoice_id, payment_method_id, chain_id, asset_type, asset_symbol, payment_address, amount) \
         VALUES ($1, $2, 'ETH-11155111', 'eip155:11155111', 'native', 'ETH', $3, 1)",
    )
    .bind(id)
    .bind(invoice)
    .bind(address)
    .execute(pool)
    .await
    .expect("seed payment option");
    id
}

async fn seed_watched_address(pool: &PgPool, invoice: &str, payment_option: Uuid, address: &str) {
    sqlx::query(
        "INSERT INTO watched_addresses \
         (invoice_id, payment_option_id, address, chain_id, expires_at) \
         VALUES ($1, $2, $3, 'eip155:11155111', now() + interval '1 hour')",
    )
    .bind(invoice)
    .bind(payment_option)
    .bind(address)
    .execute(pool)
    .await
    .expect("seed watched address");
}

async fn seed_payout(pool: &PgPool, store: Uuid) {
    sqlx::query(
        "INSERT INTO payouts (id, store_id, destination_address, chain_id, asset_symbol, amount) \
         VALUES ($1, $2, '0x0000000000000000000000000000000000000000', 'eip155:11155111', 'ETH', '1')",
    )
    .bind(Uuid::new_v4())
    .bind(store)
    .execute(pool)
    .await
    .expect("seed payout");
}

async fn cleanup(pool: &PgPool, users: &[Uuid]) {
    for user in users {
        let _ = sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user)
            .execute(pool)
            .await;
    }
}

/// A payout blocks a user's own cascading delete (`stores -> payouts` is not
/// `ON DELETE CASCADE`, see `data_service::account_deletion`), so a leftover
/// payout from a test that failed partway must be cleared before `cleanup`
/// can remove the user underneath it.
async fn clear_payouts_for_store(pool: &PgPool, store: Uuid) {
    let _ = sqlx::query("DELETE FROM payouts WHERE store_id = $1")
        .bind(store)
        .execute(pool)
        .await;
}

fn admin_auth(id: Uuid) -> AdminAuth {
    AdminAuth(UserInfo {
        id: UserId(id),
        email: None,
        primary_wallet_address: None,
        created_at: chrono::Utc::now(),
        last_login_at: None,
        role: Role::ServerAdmin,
    })
}

fn app_state(data_service: Arc<PgDataService>) -> PgAppState<UnusedSessionService> {
    app_state_with_monitor(data_service, None)
}

fn app_state_with_monitor(
    data_service: Arc<PgDataService>,
    evm_monitor: Option<Arc<RedisEVMMonitor>>,
) -> PgAppState<UnusedSessionService> {
    PgAppState::new(
        data_service,
        Arc::new(UnusedSessionService),
        evm_monitor,
        Arc::new(NoOpRateProvider),
        Arc::new(server::services::email::NoopEmailSender),
    )
}

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

/// The whole safety property of `hard_delete_store`: it hard-deletes on a
/// live server and cannot lean on "no financial history" the way
/// `delete_user_account` does, since removing a paid synthetic invoice is the
/// point. The name check is what stands between it and a real merchant's
/// store.
#[tokio::test]
#[ignore]
async fn hard_delete_store_refuses_a_name_that_is_not_the_synthetic_shape() {
    let Some(pg) = service().await else {
        return;
    };
    let caller = seed_user(pg.pool(), "server_admin").await;
    let target = seed_user(pg.pool(), "user").await;
    let store = Store::new(format!("A Real Merchant's Shop {target}"), UserId(target));
    pg.create_store_owned_by(&store, UserId(target))
        .await
        .expect("seed store");

    let state = app_state(Arc::new(pg));

    let result = hard_delete_store(
        admin_auth(caller),
        Path(store.id.0.to_string()),
        State(state.clone()),
    )
    .await;

    let Err((status, _)) = result else {
        panic!("a non-synthetic store name must be refused");
    };
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let still_there: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stores WHERE id = $1")
        .bind(store.id.0)
        .fetch_one(state.data_service.pool())
        .await
        .expect("count store");
    assert_eq!(still_there, 1, "the refusal must not have deleted anything");

    cleanup(state.data_service.pool(), &[caller, target]).await;
}

/// The success path the synthetic-payment job's own cleanup and the store
/// backfill sweep both depend on: a matching-name store, along with the
/// invoice and payment it carries, is actually gone afterward - not merely
/// archived.
#[tokio::test]
#[ignore]
async fn hard_delete_store_removes_a_synthetic_store_with_its_invoice_and_payment() {
    let Some(pg) = service().await else {
        return;
    };
    let caller = seed_user(pg.pool(), "server_admin").await;
    let target = seed_user(pg.pool(), "user").await;
    let store = Store::new(
        "e2e-synthetic-2026-01-01T00-00-00-000Z".to_string(),
        UserId(target),
    );
    pg.create_store_owned_by(&store, UserId(target))
        .await
        .expect("seed store");
    let invoice = seed_invoice(pg.pool(), store.id.0).await;
    seed_payment(pg.pool(), &invoice).await;

    let state = app_state(Arc::new(pg));

    let result = hard_delete_store(
        admin_auth(caller),
        Path(store.id.0.to_string()),
        State(state.clone()),
    )
    .await;

    assert_eq!(result, Ok(StatusCode::NO_CONTENT));

    let stores: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stores WHERE id = $1")
        .bind(store.id.0)
        .fetch_one(state.data_service.pool())
        .await
        .expect("count stores");
    assert_eq!(stores, 0);

    let invoices: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM invoices WHERE id = $1")
        .bind(&invoice)
        .fetch_one(state.data_service.pool())
        .await
        .expect("count invoices");
    assert_eq!(
        invoices, 0,
        "the invoice should have gone with the store, not been left behind"
    );

    let payments: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payments WHERE invoice_id = $1")
        .bind(&invoice)
        .fetch_one(state.data_service.pool())
        .await
        .expect("count payments");
    assert_eq!(payments, 0, "the payment should have cascaded away too");

    cleanup(state.data_service.pool(), &[caller, target]).await;
}

/// `ON DELETE CASCADE` does not reach `payouts` (see
/// `data_service::account_deletion`), so a store that somehow holds one -
/// which a synthetic E2E store never should - must be refused rather than
/// silently destroying it or failing halfway through the cascade.
#[tokio::test]
#[ignore]
async fn hard_delete_store_refuses_when_a_payout_exists() {
    let Some(pg) = service().await else {
        return;
    };
    let caller = seed_user(pg.pool(), "server_admin").await;
    let target = seed_user(pg.pool(), "user").await;
    let store = Store::new(
        "e2e-synthetic-2026-01-02T00-00-00-000Z".to_string(),
        UserId(target),
    );
    pg.create_store_owned_by(&store, UserId(target))
        .await
        .expect("seed store");
    seed_payout(pg.pool(), store.id.0).await;

    let state = app_state(Arc::new(pg));

    let result = hard_delete_store(
        admin_auth(caller),
        Path(store.id.0.to_string()),
        State(state.clone()),
    )
    .await;

    let Err((status, message)) = result else {
        panic!("a store holding a payout must be refused");
    };
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        message.contains("payout"),
        "the operator must see why, got: {message}"
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
    let Some(redis_url) = std::env::var("REDIS_URL").ok() else {
        return;
    };
    let monitor = RedisEVMMonitor::connect(&redis_url)
        .await
        .unwrap_or_else(|e| panic!("REDIS_URL is set but connecting failed: {e}"));

    let subscriber = RedisBridge::new(&redis_url, EVENTS_CHANNEL, COMMANDS_CHANNEL)
        .await
        .expect("connect a second bridge to observe published commands");
    let mut commands = subscriber
        .subscribe_commands()
        .await
        .expect("subscribe to the commands channel");

    let caller = seed_user(pg.pool(), "server_admin").await;
    let target = seed_user(pg.pool(), "user").await;
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
    let Some(redis_url) = std::env::var("REDIS_URL").ok() else {
        return;
    };
    let monitor = RedisEVMMonitor::connect(&redis_url)
        .await
        .unwrap_or_else(|e| panic!("REDIS_URL is set but connecting failed: {e}"));

    let subscriber = RedisBridge::new(&redis_url, EVENTS_CHANNEL, COMMANDS_CHANNEL)
        .await
        .expect("connect a second bridge to observe published commands");
    let mut commands = subscriber
        .subscribe_commands()
        .await
        .expect("subscribe to the commands channel");

    let caller = seed_user(pg.pool(), "server_admin").await;
    let target = seed_user(pg.pool(), "user").await;
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
