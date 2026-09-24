#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Review finding, fixed: `require_store_settings_permission` is the shared
//! gate behind ~16 handlers across payment_methods.rs, settings.rs,
//! token_policy.rs, wallets.rs and webhooks.rs, but every existing test that
//! reaches it does so via `admin_store_scoped_user` (`Role::ServerAdmin`,
//! `key_scope: None`), which returns before the added
//! `key_grants_store_permission` line ever runs. `api_key_store_scope.rs`
//! drives a real narrowed key through `create_invoice`/`update_store`, but
//! those have their own inline permission checks, not this shared helper -
//! so the intersection line behind the largest group of call sites had zero
//! coverage anywhere. These tests drive `get_store_settings`, one of that
//! helper's callers, with a real non-admin owner and a real `Some(scope)`,
//! in both directions plus the store-id-scoping case.

use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use sqlx::PgPool;
use uuid::Uuid;

use auth::{
    Policies, Result as AuthResult, Role, Session, SessionId, SessionService, Store, UserId,
    UserInfo,
};
use data_service::PgDataService;
use data_service::store_creation::StoreCreationWriter;
use rates::NoOpRateProvider;
use server::api::StoreScopedUser;
use server::api::stores::get_store_settings;
use server::state::PgAppState;

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

/// A key scoped to `canmodifystoresettings` reaches past
/// `require_store_settings_permission` and reads the store's settings.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_modify_settings_can_read_store_settings() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect(
            "seed store owned by user, with the Owner role's canmodifystoresettings permission",
        );

    let state = app_state(Arc::new(pg));

    let result = get_store_settings(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_MODIFY_SETTINGS.to_string()]),
        ),
        State(state),
        Path(store.id.0),
    )
    .await;

    assert!(
        result.is_ok(),
        "a key scoped to canmodifystoresettings must be able to read store settings: {:?}",
        result.err()
    );
}

/// The other direction: a key scoped only to `cancreateinvoice` is refused by
/// `require_store_settings_permission`, even though the owner's own role
/// grants `canmodifystoresettings` too - proving the check is a real
/// intersection with the key's narrower scope, not just the owner's role.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_create_invoice_is_refused_store_settings_read() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");

    let state = app_state(Arc::new(pg));

    let result = get_store_settings(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_CREATE_INVOICE.to_string()]),
        ),
        State(state),
        Path(store.id.0),
    )
    .await;

    assert_eq!(
        result.err(),
        Some(StatusCode::FORBIDDEN),
        "a key not scoped to canmodifystoresettings must be refused by require_store_settings_permission"
    );
}

/// `policy:storeId` scoping through the same shared helper: a key scoped to
/// store A's settings permission must not grant it on store B.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_one_store_is_refused_store_settings_on_another() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store_a = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    let store_b = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store_a, UserId(owner))
        .await
        .expect("seed store a");
    pg.create_store_owned_by(&store_b, UserId(owner))
        .await
        .expect("seed store b");

    let state = app_state(Arc::new(pg));

    let scoped_to_store_a = vec![format!(
        "{}:{}",
        Policies::STORE_MODIFY_SETTINGS,
        store_a.id.0
    )];

    let result = get_store_settings(
        StoreScopedUser(user_info(owner), Some(scoped_to_store_a)),
        State(state),
        Path(store_b.id.0),
    )
    .await;

    assert_eq!(
        result.err(),
        Some(StatusCode::FORBIDDEN),
        "a key scoped to store A's settings permission must be refused on store B"
    );
}
