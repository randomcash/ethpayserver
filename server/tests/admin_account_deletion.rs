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

use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use sqlx::PgPool;
use uuid::Uuid;

use auth::{
    Result as AuthResult, Role, Session, SessionId, SessionService, Store, UserId, UserInfo,
};
use data_service::PgDataService;
use data_service::store_creation::StoreCreationWriter;
use rates::NoOpRateProvider;
use server::api::AdminAuth;
use server::api::admin::{delete_user_account, list_user_stores};
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
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .ok()?;
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

async fn cleanup(pool: &PgPool, users: &[Uuid]) {
    for user in users {
        let _ = sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user)
            .execute(pool)
            .await;
    }
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
    PgAppState::new(
        data_service,
        Arc::new(UnusedSessionService),
        None::<Arc<RedisEVMMonitor>>,
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
