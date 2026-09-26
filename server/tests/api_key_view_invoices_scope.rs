#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Review finding, fixed: `list_invoices`, `get_invoice`, `get_invoice_payments`,
//! `get_invoice_status`, `list_payments`, `get_payment`, both CSV exports and
//! `lookup_by_tx_hash` all read invoice/payment data gated only by store
//! membership - none of them consulted the authenticating key's stored scope
//! at all. A key narrowed to `cancreateinvoice` alone (the ticket's own
//! canonical example - "create a test invoice but not install a plugin")
//! could still list and read every invoice and payment on every store its
//! owner belongs to. These tests drive the two most direct of those paths,
//! `list_invoices` and `get_invoice`, with a real `Some(scope)` through the
//! actual handler bodies, in both directions - a key without `canviewinvoices`
//! refused, a key with it let through - the same "both directions, or the
//! test proves nothing" bar the store-settings tests hold themselves to.

use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::Utc;
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
use server::api::invoices::{ListInvoicesQuery, get_invoice, list_invoices};
use server::services::RedisEVMMonitor;
use server::state::PgAppState;
use types::{InvoiceData, InvoiceId, InvoiceWriter, StoreId};

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
        created_at: Utc::now(),
        last_login_at: None,
        role: Role::User,
    }
}

fn test_invoice(store_id: StoreId) -> InvoiceData {
    InvoiceData {
        id: InvoiceId::new(),
        store_id,
        currency: "USD".to_string(),
        status: types::InvoiceStatus::Pending,
        amount: "100.00".to_string(),
        amount_received: "0".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        customer_email: None,
        extra: None,
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

fn list_query(store_id: Uuid) -> ListInvoicesQuery {
    ListInvoicesQuery {
        store_id: Some(store_id),
        status: None,
        currency: None,
        search: None,
        limit: None,
        offset: None,
    }
}

/// A key scoped only to `cancreateinvoice` must not be able to list invoices
/// on a store its owner belongs to - `canviewinvoices` is a different grant.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_create_invoice_is_refused_list_invoices() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");

    let state = app_state(Arc::new(pg));

    let result = list_invoices(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_CREATE_INVOICE.to_string()]),
        ),
        State(state),
        Query(list_query(store.id.0)),
    )
    .await;

    let Err(err) = result else {
        panic!("a key not scoped to canviewinvoices must be refused list_invoices");
    };
    assert_eq!(
        err.into_response().status(),
        StatusCode::FORBIDDEN,
        "expected the key's narrower scope to refuse the list"
    );
}

/// The other direction: a key scoped to `canviewinvoices` reaches past the
/// permission check and lists the store's (empty) invoices.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_view_invoices_can_list_invoices() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");

    let state = app_state(Arc::new(pg));

    let result = list_invoices(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_VIEW_INVOICES.to_string()]),
        ),
        State(state),
        Query(list_query(store.id.0)),
    )
    .await;

    assert!(
        result.is_ok(),
        "a key scoped to canviewinvoices must be able to list invoices: {:?}",
        result.err()
    );
}

/// Same pair, on `get_invoice`: a key scoped only to `cancreateinvoice` is
/// refused reading a real invoice on a store its owner belongs to.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_create_invoice_is_refused_get_invoice() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");
    let invoice = test_invoice(store.id);
    InvoiceWriter::upsert(&pg, &invoice)
        .await
        .expect("seed invoice");

    let state = app_state(Arc::new(pg));

    let result = get_invoice(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_CREATE_INVOICE.to_string()]),
        ),
        State(state),
        Path(invoice.id.0.clone()),
    )
    .await;

    assert_eq!(
        result.err(),
        Some(StatusCode::FORBIDDEN),
        "a key scoped only to cancreateinvoice must not be able to read the invoice"
    );
}

/// A key scoped to `canviewinvoices` can read the same invoice.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_view_invoices_can_get_invoice() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");
    let invoice = test_invoice(store.id);
    InvoiceWriter::upsert(&pg, &invoice)
        .await
        .expect("seed invoice");

    let state = app_state(Arc::new(pg));

    let result = get_invoice(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_VIEW_INVOICES.to_string()]),
        ),
        State(state),
        Path(invoice.id.0.clone()),
    )
    .await;

    assert!(
        result.is_ok(),
        "a key scoped to canviewinvoices must be able to read the invoice: {:?}",
        result.err()
    );
}

/// An unscoped (pre-existing) key keeps reading invoices exactly as before -
/// membership alone still governs it, same as session auth.
#[tokio::test]
#[ignore]
async fn a_preexisting_unscoped_key_still_lists_invoices() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");

    let state = app_state(Arc::new(pg));

    let result = list_invoices(
        StoreScopedUser(user_info(owner), None),
        State(state),
        Query(list_query(store.id.0)),
    )
    .await;

    assert!(
        result.is_ok(),
        "an unscoped key must list invoices exactly as before: {:?}",
        result.err()
    );
}
