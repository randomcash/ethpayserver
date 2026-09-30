#![allow(clippy::unwrap_used, clippy::expect_used)]
//! HTTP-level coverage for `GET /stores/{id}/wallet?payment_method_id=...`.
//!
//! Review finding, fixed: the data-service test proving the fix
//! (`a_pin_outlives_a_primary_change_and_diverges_from_the_store` in
//! `data-service/src/postgres/integration_tests/wallet.rs`) calls
//! `StorePaymentMethodReader`/`StorePaymentMethodWriter`/`WalletWriter`
//! directly - it never reaches `get_store_wallet`'s own branching: the
//! cross-store 404, the unresolvable-method 404, and the `is_override`
//! comparison the handler computes against `resolve_store_wallet`. This calls
//! the handler itself against a real `PgAppState`, the same pattern
//! `email_change_smtp_gate.rs` uses.
//!
//! Needs a real Postgres and is `#[ignore]`d, matching the convention
//! `data-service`'s own DB-backed tests use: set `DATABASE_URL` and run with
//! `--ignored`. Skips (rather than failing) when it is unset, same as those
//! tests, so the default `cargo test` run stays DB-free.

use std::sync::Arc;

use async_trait::async_trait;
use auth::{
    Result as AuthResult, Role, Session, SessionId, SessionService, Store, UserId, UserInfo,
};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use data_service::store_creation::StoreCreationWriter;
use data_service::{PgDataService, StorePaymentMethodWriter, WalletWriter};
use rates::NoOpRateProvider;
use server::api::StoreScopedUser;
use server::api::stores::{StoreWalletQuery, StoreWalletResult, get_store_wallet};
use server::state::PgAppState;
use sqlx::PgPool;
use types::ChainId;
use uuid::Uuid;

const EVM: &str = types::NAMESPACE_EIP155;

/// Not exercised: `get_store_wallet` never calls back into session
/// management, only reads the already-authenticated `UserInfo` this test
/// constructs directly.
struct UnusedSessionService;

#[async_trait]
impl SessionService for UnusedSessionService {
    async fn validate_session(&self, _session_id: SessionId) -> AuthResult<(UserInfo, Session)> {
        unimplemented!("not exercised by get_store_wallet")
    }
    async fn logout(&self, _session_id: SessionId) -> AuthResult<()> {
        unimplemented!("not exercised by get_store_wallet")
    }
    async fn logout_all(&self, _session_id: SessionId) -> AuthResult<()> {
        unimplemented!("not exercised by get_store_wallet")
    }
    async fn cleanup_stale_sessions(&self) -> AuthResult<u64> {
        unimplemented!("not exercised by get_store_wallet")
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

async fn seed_user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, kdf_params, encrypted_symmetric_key, \
         recovery_verification_hash, kdf_salt_identifier) \
         VALUES ($1, \
             '{\"algorithm\":\"argon2id\",\"memory_kb\":65536,\"iterations\":3,\"parallelism\":4,\"salt\":\"\"}'::jsonb, \
             '{\"ciphertext\":\"\",\"iv\":\"\",\"mac\":\"\"}'::jsonb, \
             'h', 'passkey:' || $1::text)",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("seed user");
    id
}

/// A key no other test is using - the repository only checks it's unique,
/// never that it's valid BIP-32.
fn unique_xpub(tag: &str) -> String {
    format!("xpub-{tag}-{}", Uuid::new_v4())
}

fn user_info(id: Uuid) -> UserInfo {
    UserInfo {
        id: UserId(id),
        email: None,
        primary_wallet_address: None,
        created_at: chrono::Utc::now(),
        last_login_at: None,
        role: Role::User,
    }
}

fn app_state(data_service: Arc<PgDataService>) -> PgAppState<UnusedSessionService> {
    PgAppState::new(
        data_service,
        Arc::new(UnusedSessionService),
        None,
        Arc::new(NoOpRateProvider),
        Arc::new(server::services::email::NoopEmailSender),
    )
}

async fn wallet_for_method(
    state: &PgAppState<UnusedSessionService>,
    owner: Uuid,
    store_id: Uuid,
    method_id: Uuid,
) -> Result<server::api::stores::MethodWalletResponse, StatusCode> {
    let result = get_store_wallet(
        StoreScopedUser(user_info(owner), None),
        State(state.clone()),
        Path(store_id),
        Query(StoreWalletQuery {
            namespace: None,
            payment_method_id: Some(method_id),
        }),
    )
    .await?;

    match result {
        StoreWalletResult::Method(body) => Ok(body),
        StoreWalletResult::Bare(_) => {
            panic!("payment_method_id was set - the handler must take the method branch")
        }
    }
}

/// The property the ticket exists for, reached through the endpoint rather
/// than the repository: once a pinned method's wallet and the store's bare
/// resolution diverge, `?payment_method_id=...` must report the pin, and
/// `differs_from_store_wallet` must say so - `is_override` must not, since no
/// override is configured on the store at all.
#[tokio::test]
#[ignore]
async fn a_pinned_methods_wallet_outlives_a_primary_change_through_the_endpoint() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user, with the Owner role's canviewstoresettings");

    let xpub_a = unique_xpub("a");
    let xpub_b = unique_xpub("b");

    let method = StorePaymentMethodWriter::create_payment_method(
        &pg,
        store.id.0,
        &ChainId::evm(1),
        None,
        "ETH",
        18,
        Some(&xpub_a),
    )
    .await
    .expect("create method pinned by xpub");
    let pinned_wallet = method.wallet_id.expect("resolves");

    let second = WalletWriter::create_wallet(&pg, owner, EVM, &xpub_b, Some("second"))
        .await
        .expect("create second wallet");
    WalletWriter::set_primary_wallet(&pg, owner, second.id)
        .await
        .expect("promote second wallet");

    let state = app_state(Arc::new(pg));

    let scoped = wallet_for_method(&state, owner, store.id.0, method.id)
        .await
        .expect("method-scoped read");
    assert_eq!(
        scoped.store_wallet.wallet.id, pinned_wallet,
        "the method-scoped endpoint must report the pin, not the new primary"
    );
    assert!(
        scoped.differs_from_store_wallet,
        "the method's resolution now diverges from the store's bare walk - \
         differs_from_store_wallet must say so"
    );
    assert!(
        !scoped.store_wallet.is_override,
        "no override is configured on the store - only the account primary \
         moved - so is_override must not claim one, even though the \
         method-scoped wallet differs from the bare walk"
    );

    let store_bare = match get_store_wallet(
        StoreScopedUser(user_info(owner), None),
        State(state),
        Path(store.id.0),
        Query(StoreWalletQuery {
            namespace: None,
            payment_method_id: None,
        }),
    )
    .await
    .expect("store-bare read")
    {
        StoreWalletResult::Bare(body) => body,
        StoreWalletResult::Method(_) => panic!("no payment_method_id - must take the bare branch"),
    };
    assert_eq!(
        store_bare.wallet.id, second.id,
        "the bare form is documented to follow the primary - confirming the \
         two forms actually disagree, which is the whole point of asking for \
         the method-scoped one"
    );
    assert!(
        !store_bare.is_override,
        "the bare form's is_override must also say no override is configured"
    );
}

/// A method that belongs to a different store must 404, the same as any
/// other cross-store lookup in this API - not leak which wallet it resolves
/// to.
#[tokio::test]
#[ignore]
async fn a_payment_method_from_another_store_is_not_found() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store_a = Store::new(format!("store-a-{}", Uuid::new_v4()), UserId(owner));
    let store_b = Store::new(format!("store-b-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store_a, UserId(owner))
        .await
        .expect("seed store a");
    pg.create_store_owned_by(&store_b, UserId(owner))
        .await
        .expect("seed store b");

    let method = StorePaymentMethodWriter::create_payment_method(
        &pg,
        store_b.id.0,
        &ChainId::evm(1),
        None,
        "ETH",
        18,
        Some(&unique_xpub("cross-store")),
    )
    .await
    .expect("create method on store b");

    let state = app_state(Arc::new(pg));
    let result = wallet_for_method(&state, owner, store_a.id.0, method.id).await;

    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "a method scoped to another store must not resolve as though it \
         belonged to this one"
    );
}

/// A method with nothing to derive from refuses to answer rather than
/// inventing a wallet - the same "visible but cannot allocate" contract the
/// repository enforces, now checked through the endpoint the ticket's e2e
/// test actually calls.
#[tokio::test]
#[ignore]
async fn an_unresolvable_method_is_not_found() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store");

    let method = StorePaymentMethodWriter::create_payment_method(
        &pg,
        store.id.0,
        &ChainId::evm(1),
        None,
        "ETH",
        18,
        Some(&unique_xpub("unresolvable")),
    )
    .await
    .expect("create method");

    sqlx::query("UPDATE store_payment_methods SET wallet_id = NULL WHERE id = $1")
        .bind(method.id)
        .execute(pg.pool())
        .await
        .expect("unpin method");
    sqlx::query("UPDATE wallets SET is_primary = FALSE WHERE user_id = $1")
        .bind(owner)
        .execute(pg.pool())
        .await
        .expect("clear primary");

    let state = app_state(Arc::new(pg));
    let result = wallet_for_method(&state, owner, store.id.0, method.id).await;

    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "with nothing to derive from, the endpoint must refuse rather than \
         guess a wallet"
    );
}
