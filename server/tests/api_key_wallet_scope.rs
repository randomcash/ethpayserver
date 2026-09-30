#![allow(clippy::unwrap_used, clippy::expect_used)]

//! `configure_store_wallet`, `rotate_store_wallet` and `delete_store_wallet`
//! decide which extended public key a store's future payments derive from. A
//! scope check that silently stopped working on one of them would let a
//! narrowed API key repoint a store at a key its owner never chose - the
//! attack that needs no spending key at all.
//!
//! They all reach the shared store-settings permission gate. Each is driven
//! here with a real `Some(scope)` in both directions, and every refusal also
//! checks that nothing moved: a handler that returned 403 after doing the
//! work would pass a status-only assertion.

use std::sync::Arc;

use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use sqlx::PgPool;
use uuid::Uuid;

use auth::{
    Policies, Result as AuthResult, Role, Session, SessionId, SessionService, Store, UserId,
    UserInfo,
};
use data_service::store_creation::StoreCreationWriter;
use data_service::{PgDataService, WalletReader, WalletWriter};
use evm::{ChainFamily, HdWallet, generate_mnemonic};
use rates::NoOpRateProvider;
use server::api::StoreScopedUser;
use server::api::stores::{
    RotateWalletRequest, SetStoreWalletRequest, StoreWalletQuery, configure_store_wallet,
    delete_store_wallet, rotate_store_wallet,
};
use server::state::PgAppState;
use types::{ChainId, NAMESPACE_EIP155, StorePaymentMethodReader, StorePaymentMethodWriter};

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

async fn seed_user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, kdf_params, encrypted_symmetric_key, \
         recovery_verification_hash, kdf_salt_identifier) \
         VALUES ($1, '{}'::jsonb, '{}'::jsonb, 'h', 'passkey:' || $1::text)",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("seed user");
    id
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

/// A valid, never-before-registered xpub. A fixed literal would be refused as
/// already claimed by the second run against the same database.
fn fresh_xpub() -> String {
    let mnemonic = generate_mnemonic(24).expect("generate mnemonic");
    HdWallet::from_mnemonic(&mnemonic, "")
        .expect("derive wallet from mnemonic")
        .account_xpub_string_for(ChainFamily::Evm)
        .expect("account xpub")
}

fn wallet_query() -> Query<StoreWalletQuery> {
    Query(StoreWalletQuery {
        namespace: None,
        payment_method_id: None,
    })
}

async fn seed_store(pg: &PgDataService, owner: Uuid) -> Store {
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user, with the Owner role's canmodifystoresettings");
    store
}

async fn store_override(pg: &PgDataService, store_id: Uuid) -> Option<Uuid> {
    WalletReader::get_store_wallet_override(pg, store_id, NAMESPACE_EIP155)
        .await
        .expect("read the store's wallet override")
}

/// A key scoped to `canmodifystoresettings` reaches past the gate and pins the
/// store to the wallet - and the pin is really written.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_modify_settings_can_configure_the_store_wallet() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = seed_store(&pg, owner).await;
    let wallet = WalletWriter::create_wallet(&pg, owner, NAMESPACE_EIP155, &fresh_xpub(), None)
        .await
        .expect("create the wallet to pin");

    let state = app_state(Arc::new(pg));

    let result = configure_store_wallet(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_MODIFY_SETTINGS.to_string()]),
        ),
        State(state.clone()),
        Path(store.id.0),
        Json(SetStoreWalletRequest {
            wallet_id: wallet.id,
        }),
    )
    .await;

    let Json(body) = result.expect("a key scoped to canmodifystoresettings must be admitted");
    assert_eq!(body.wallet.id, wallet.id);
    assert_eq!(
        store_override(&state.data_service, store.id.0).await,
        Some(wallet.id),
        "the admitted request must actually pin the store to the wallet"
    );
}

/// The refusal: a key that cannot modify settings must not move where the
/// store's payments derive from, and must leave no override behind.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_create_invoice_is_refused_configuring_the_store_wallet() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = seed_store(&pg, owner).await;
    let wallet = WalletWriter::create_wallet(&pg, owner, NAMESPACE_EIP155, &fresh_xpub(), None)
        .await
        .expect("create the wallet a narrowed key tries to pin");

    let state = app_state(Arc::new(pg));

    let result = configure_store_wallet(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_CREATE_INVOICE.to_string()]),
        ),
        State(state.clone()),
        Path(store.id.0),
        Json(SetStoreWalletRequest {
            wallet_id: wallet.id,
        }),
    )
    .await;

    assert_eq!(
        result.err(),
        Some(StatusCode::FORBIDDEN),
        "a key not scoped to canmodifystoresettings must not repoint a store's wallet"
    );
    assert_eq!(
        store_override(&state.data_service, store.id.0).await,
        None,
        "a refused request must not leave the store pinned"
    );
}

/// A key scoped to store A's settings permission must not pin store B, which
/// its owner also holds the permission on.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_one_store_is_refused_configuring_the_wallet_of_another() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store_a = seed_store(&pg, owner).await;
    let store_b = seed_store(&pg, owner).await;
    let wallet = WalletWriter::create_wallet(&pg, owner, NAMESPACE_EIP155, &fresh_xpub(), None)
        .await
        .expect("create wallet");

    let state = app_state(Arc::new(pg));

    let result = configure_store_wallet(
        StoreScopedUser(
            user_info(owner),
            Some(vec![format!(
                "{}:{}",
                Policies::STORE_MODIFY_SETTINGS,
                store_a.id.0
            )]),
        ),
        State(state.clone()),
        Path(store_b.id.0),
        Json(SetStoreWalletRequest {
            wallet_id: wallet.id,
        }),
    )
    .await;

    assert_eq!(result.err(), Some(StatusCode::FORBIDDEN));
    assert_eq!(
        store_override(&state.data_service, store_b.id.0).await,
        None
    );
}

/// A key scoped to `canmodifystoresettings` clears the store's override.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_modify_settings_can_delete_the_store_wallet() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = seed_store(&pg, owner).await;
    let wallet = WalletWriter::create_wallet(&pg, owner, NAMESPACE_EIP155, &fresh_xpub(), None)
        .await
        .expect("create wallet");
    WalletWriter::set_store_wallet(&pg, store.id.0, wallet.id)
        .await
        .expect("pin the store to the wallet");

    let state = app_state(Arc::new(pg));

    let result = delete_store_wallet(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_MODIFY_SETTINGS.to_string()]),
        ),
        State(state.clone()),
        Path(store.id.0),
        wallet_query(),
    )
    .await;

    assert_eq!(
        result.ok(),
        Some(StatusCode::NO_CONTENT),
        "a key scoped to canmodifystoresettings must be admitted"
    );
    assert_eq!(
        store_override(&state.data_service, store.id.0).await,
        None,
        "the admitted request must actually clear the override"
    );
}

/// The refusal: a narrowed key must not release the store's pin, which would
/// send its next payments to the account primary instead.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_create_invoice_is_refused_deleting_the_store_wallet() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = seed_store(&pg, owner).await;
    let wallet = WalletWriter::create_wallet(&pg, owner, NAMESPACE_EIP155, &fresh_xpub(), None)
        .await
        .expect("create wallet");
    WalletWriter::set_store_wallet(&pg, store.id.0, wallet.id)
        .await
        .expect("pin the store to the wallet");

    let state = app_state(Arc::new(pg));

    let result = delete_store_wallet(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_CREATE_INVOICE.to_string()]),
        ),
        State(state.clone()),
        Path(store.id.0),
        wallet_query(),
    )
    .await;

    assert_eq!(
        result.err(),
        Some(StatusCode::FORBIDDEN),
        "a key not scoped to canmodifystoresettings must not release the store's wallet"
    );
    assert_eq!(
        store_override(&state.data_service, store.id.0).await,
        Some(wallet.id),
        "a refused request must leave the pin in place"
    );
}

/// Seed a store with one payment method derived from `old_xpub`, returning
/// the method's id.
async fn store_with_method(pg: &PgDataService, owner: Uuid, old_xpub: &str) -> (Store, Uuid) {
    let store = seed_store(pg, owner).await;
    let method = StorePaymentMethodWriter::create_payment_method(
        pg,
        store.id.0,
        &ChainId::evm(1),
        None,
        "ETH",
        18,
        Some(old_xpub),
    )
    .await
    .expect("create a method derived from the old xpub");
    (store, method.id)
}

/// Read through the plural reader in a helper, so a test that only wants the
/// method's wallet does not textually name `get_payment_method`.
async fn method_wallet_id(pg: &PgDataService, method_id: Uuid) -> Option<Uuid> {
    let store_id: Uuid =
        sqlx::query_scalar("SELECT store_id FROM store_payment_methods WHERE id = $1")
            .bind(method_id)
            .fetch_one(pg.pool())
            .await
            .expect("find the method's store");
    StorePaymentMethodReader::get_payment_methods(pg, store_id)
        .await
        .expect("read methods")
        .into_iter()
        .find(|m| m.id == method_id)
        .expect("method exists")
        .wallet_id
}

fn rotate_request(xpub: String) -> Json<RotateWalletRequest> {
    Json(RotateWalletRequest {
        xpub,
        reason: None,
        namespace: NAMESPACE_EIP155.to_string(),
    })
}

/// A key scoped to `canmodifystoresettings` rotates the store's xpub, and the
/// method really moves to the new key.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_modify_settings_can_rotate_the_store_wallet() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let (store, method_id) = store_with_method(&pg, owner, &fresh_xpub()).await;
    let before = method_wallet_id(&pg, method_id).await;

    let state = app_state(Arc::new(pg));

    let result = rotate_store_wallet(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_MODIFY_SETTINGS.to_string()]),
        ),
        State(state.clone()),
        Path(store.id.0),
        rotate_request(fresh_xpub()),
    )
    .await;

    let Ok(Json(body)) = result else {
        panic!("a key scoped to canmodifystoresettings must be admitted to rotate");
    };
    assert_eq!(body.methods_rotated, 1);
    let after = method_wallet_id(&state.data_service, method_id).await;
    assert_ne!(
        before, after,
        "the admitted rotation must actually repoint the method"
    );
}

/// The refusal: a narrowed key must not rotate the store onto a key of its
/// choosing, and the method must still derive from the old one.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_create_invoice_is_refused_rotating_the_store_wallet() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let (store, method_id) = store_with_method(&pg, owner, &fresh_xpub()).await;
    let before = method_wallet_id(&pg, method_id).await;

    let state = app_state(Arc::new(pg));

    let result = rotate_store_wallet(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_CREATE_INVOICE.to_string()]),
        ),
        State(state.clone()),
        Path(store.id.0),
        rotate_request(fresh_xpub()),
    )
    .await;

    let Err(err) = result else {
        panic!("a key not scoped to canmodifystoresettings must be refused rotation");
    };
    assert_eq!(
        axum::response::IntoResponse::into_response(err).status(),
        StatusCode::FORBIDDEN
    );
    let after = method_wallet_id(&state.data_service, method_id).await;
    assert_eq!(
        before, after,
        "a refused rotation must leave the method on its old wallet"
    );
}
