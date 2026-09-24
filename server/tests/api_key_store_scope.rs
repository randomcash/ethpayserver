#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Review finding, fixed: every existing test that calls a `StoreScopedUser`
//! handler - `plugin_invoice_creation_filter.rs`, `stores::tests` - passes
//! `None` for the key scope, so none of them can tell "the intersection is
//! wired correctly at this call site" apart from "the second operand is
//! never consulted". The ticket's own Verify section is explicit about why
//! that matters: "A key granted only `cancreateinvoice` creates an invoice
//! and is refused store-settings changes. Both directions, or the test
//! proves nothing." These tests drive `create_invoice` and `update_store`
//! with a real `Some(scope)` through the actual handler bodies against a
//! real store/role seed, in both directions, plus the store-id-scoping and
//! owner-revocation cases the same section asks for.

use std::sync::Arc;

use async_trait::async_trait;
use axum::Json;
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
use server::api::invoices::{CreateInvoiceRequest, create_invoice};
use server::api::stores::{UpdateStoreRequest, update_store};
use server::services::RedisEVMMonitor;
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

fn invoice_request(store_id: Uuid) -> CreateInvoiceRequest {
    CreateInvoiceRequest {
        store_id,
        currency: "USD".to_string(),
        amount: "10.00".to_string(),
        expiration_seconds: None,
        metadata: None,
        customer_email: None,
        webhook_url: None,
        redirect_url: None,
    }
}

fn no_op_update() -> UpdateStoreRequest {
    UpdateStoreRequest {
        name: None,
        website: None,
    }
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

/// A key scoped only to `cancreateinvoice` reaches past the permission check
/// on `create_invoice`. It still fails downstream (no payment methods
/// configured on a freshly seeded store) - the point is *which* check it
/// fails, not full success, so the assertion is "not FORBIDDEN".
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_create_invoice_reaches_past_the_permission_check() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user, with the Owner role's cancreateinvoice permission");

    let state = app_state(Arc::new(pg));

    let result = create_invoice(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_CREATE_INVOICE.to_string()]),
        ),
        State(state),
        Json(invoice_request(store.id.0)),
    )
    .await;

    let Err((status, Json(body))) = result else {
        panic!("expected a downstream failure past the permission check (no payment methods)");
    };
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "a key scoped to cancreateinvoice must pass the permission check, got {body:?}"
    );
}

/// The same owner, same store, but the key is scoped to a different
/// permission than the one `create_invoice` checks. The owner's own role
/// grants `cancreateinvoice` - only the key's narrower scope should refuse
/// it, proving the check is a real intersection and not "key scope present
/// implies allowed".
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_something_else_is_refused_invoice_creation() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");

    let state = app_state(Arc::new(pg));

    let result = create_invoice(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_MODIFY_SETTINGS.to_string()]),
        ),
        State(state),
        Json(invoice_request(store.id.0)),
    )
    .await;

    let Err((status, Json(body))) = result else {
        panic!("a key not scoped to cancreateinvoice must be refused");
    };
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "expected the key's narrower scope to refuse invoice creation, got {body:?}"
    );
}

/// The other direction of the same pair: a key scoped only to
/// `canmodifystoresettings` can update the store, even though its owner also
/// has `cancreateinvoice`.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_modify_settings_can_update_the_store() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");

    let state = app_state(Arc::new(pg));

    let result = update_store(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_MODIFY_SETTINGS.to_string()]),
        ),
        State(state),
        Path(store.id.0),
        Json(no_op_update()),
    )
    .await;

    assert!(
        result.is_ok(),
        "a key scoped to canmodifystoresettings must be able to update the store: {:?}",
        result.err()
    );
}

/// A key granted only `cancreateinvoice` is refused store-settings changes -
/// the ticket's own bidirectional example, in the direction the invoice test
/// above doesn't cover.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_create_invoice_is_refused_store_settings_changes() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");

    let state = app_state(Arc::new(pg));

    let result = update_store(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_CREATE_INVOICE.to_string()]),
        ),
        State(state),
        Path(store.id.0),
        Json(no_op_update()),
    )
    .await;

    assert_eq!(
        result.err(),
        Some(StatusCode::FORBIDDEN),
        "a key scoped only to cancreateinvoice must not be able to update store settings"
    );
}

/// `policy:storeId` scoping: a key scoped to store A must not grant the same
/// permission on store B, even though the same owner holds it on both.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_one_store_is_refused_on_another() {
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
        Policies::STORE_CREATE_INVOICE,
        store_a.id.0
    )];

    let result = create_invoice(
        StoreScopedUser(user_info(owner), Some(scoped_to_store_a)),
        State(state),
        Json(invoice_request(store_b.id.0)),
    )
    .await;

    let Err((status, Json(body))) = result else {
        panic!("a key scoped to store A must be refused on store B");
    };
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "expected store-scoped key to be refused on a different store, got {body:?}"
    );
}

/// Intersection, not union: an `unrestricted` key (`None` scope - the same
/// value every key issued before this feature carries) is still refused once
/// its owner's own store access is gone. The key was never touched; only the
/// owner's membership row was removed.
#[tokio::test]
#[ignore]
async fn revoking_the_owners_store_access_refuses_an_unrestricted_key_too() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");

    sqlx::query("DELETE FROM user_stores WHERE user_id = $1 AND store_id = $2")
        .bind(owner)
        .bind(store.id.0)
        .execute(pg.pool())
        .await
        .expect("revoke the owner's store membership");

    let state = app_state(Arc::new(pg));

    let result = create_invoice(
        StoreScopedUser(user_info(owner), None),
        State(state),
        Json(invoice_request(store.id.0)),
    )
    .await;

    let Err((status, Json(body))) = result else {
        panic!("an unrestricted key must lose access when its owner's store access is revoked");
    };
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "expected revoked owner access to refuse even an unrestricted key, got {body:?}"
    );
}

/// A key issued before this feature existed (`permissions` is genuinely
/// unset, not an empty selection) keeps working exactly as before: unscoped
/// still means "everything the owner can reach".
#[tokio::test]
#[ignore]
async fn a_preexisting_unscoped_key_still_creates_invoices() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");

    let state = app_state(Arc::new(pg));

    let result = create_invoice(
        StoreScopedUser(user_info(owner), None),
        State(state),
        Json(invoice_request(store.id.0)),
    )
    .await;

    let Err((status, Json(body))) = result else {
        panic!("expected a downstream failure past the permission check (no payment methods)");
    };
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "an unscoped (pre-existing) key must not be refused on the permission check, got {body:?}"
    );
}
