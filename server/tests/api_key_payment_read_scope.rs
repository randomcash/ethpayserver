#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Review finding, fixed: `get_payment`, `get_invoice_payments`,
//! `get_invoice_status` (via the shared `get_invoice_with_permission`) and
//! `lookup_by_tx_hash` each gained their own inline `key_grants_store_permission`
//! check, but every test touching them drove the call with `key_scope: None`
//! or an unrestricted seed key - never a real `Some(scope)` key through the
//! actual handler body. A copy-paste slip in any of the four (wrong policy
//! constant, wrong store id, or the guard deleted outright) would have stayed
//! green. `list_invoices`/`get_invoice` already had this coverage
//! (`api_key_view_invoices_scope.rs`); these tests do the same for the four
//! call sites that didn't, both directions on each.

use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::http::StatusCode;
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
use server::api::invoices::{
    TxHashLookupPath, get_invoice_payments, get_invoice_status, get_payment, lookup_by_tx_hash,
};
use server::services::RedisEVMMonitor;
use server::state::PgAppState;
use types::{
    AssetType, ChainId, InvoiceData, InvoiceId, InvoiceStatus, InvoiceWriter, PaymentData,
    PaymentWriter, StoreId,
};

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
        status: InvoiceStatus::Pending,
        amount: "100.00".to_string(),
        amount_received: "0".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        customer_email: None,
        extra: None,
    }
}

fn test_payment(invoice_id: &InvoiceId) -> PaymentData {
    PaymentData {
        id: Uuid::new_v4(),
        invoice_id: invoice_id.clone(),
        payment_option_id: None,
        chain_id: ChainId::evm(11155111),
        asset_type: AssetType::Native,
        amount: "1000000000000000000".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        tx_hash: format!("0x{:064x}", Uuid::new_v4().as_u128()),
        block_number: Some(1),
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: None,
        reorged: false,
        extra: None,
        credited_amount: None,
        rate_used: None,
        rate_applied_at: None,
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

/// Seeds an owner, a store they own, an invoice on it and a payment on that
/// invoice. Returns the owner id and the seeded invoice/payment.
async fn seed_store_invoice_payment(pg: &PgDataService) -> (Uuid, InvoiceData, PaymentData) {
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");
    let invoice = test_invoice(store.id);
    InvoiceWriter::upsert(pg, &invoice)
        .await
        .expect("seed invoice");
    let payment = test_payment(&invoice.id);
    PaymentWriter::upsert(pg, &payment)
        .await
        .expect("seed payment");
    (owner, invoice, payment)
}

// ---------------------------------------------------------------------------
// get_payment
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn a_key_scoped_to_create_invoice_is_refused_get_payment() {
    let Some(pg) = service().await else {
        return;
    };
    let (owner, _invoice, payment) = seed_store_invoice_payment(&pg).await;
    let state = app_state(Arc::new(pg));

    let result = get_payment(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_CREATE_INVOICE.to_string()]),
        ),
        State(state),
        Path(payment.id),
    )
    .await;

    assert_eq!(
        result.err(),
        Some(StatusCode::NOT_FOUND),
        "a key scoped only to cancreateinvoice must not be able to read the payment"
    );
}

#[tokio::test]
#[ignore]
async fn a_key_scoped_to_view_invoices_can_get_payment() {
    let Some(pg) = service().await else {
        return;
    };
    let (owner, _invoice, payment) = seed_store_invoice_payment(&pg).await;
    let state = app_state(Arc::new(pg));

    let result = get_payment(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_VIEW_INVOICES.to_string()]),
        ),
        State(state),
        Path(payment.id),
    )
    .await;

    assert!(
        result.is_ok(),
        "a key scoped to canviewinvoices must be able to read the payment: {:?}",
        result.err()
    );
}

// ---------------------------------------------------------------------------
// get_invoice_payments (shares get_invoice_with_permission with get_invoice_status)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn a_key_scoped_to_create_invoice_is_refused_get_invoice_payments() {
    let Some(pg) = service().await else {
        return;
    };
    let (owner, invoice, _payment) = seed_store_invoice_payment(&pg).await;
    let state = app_state(Arc::new(pg));

    let result = get_invoice_payments(
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
        Some(StatusCode::NOT_FOUND),
        "a key scoped only to cancreateinvoice must not be able to list the invoice's payments"
    );
}

#[tokio::test]
#[ignore]
async fn a_key_scoped_to_view_invoices_can_get_invoice_payments() {
    let Some(pg) = service().await else {
        return;
    };
    let (owner, invoice, _payment) = seed_store_invoice_payment(&pg).await;
    let state = app_state(Arc::new(pg));

    let result = get_invoice_payments(
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
        "a key scoped to canviewinvoices must be able to list the invoice's payments: {:?}",
        result.err()
    );
}

// ---------------------------------------------------------------------------
// get_invoice_status (the other caller of get_invoice_with_permission)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn a_key_scoped_to_create_invoice_is_refused_get_invoice_status() {
    let Some(pg) = service().await else {
        return;
    };
    let (owner, invoice, _payment) = seed_store_invoice_payment(&pg).await;
    let state = app_state(Arc::new(pg));

    let result = get_invoice_status(
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
        Some(StatusCode::NOT_FOUND),
        "a key scoped only to cancreateinvoice must not be able to read invoice status"
    );
}

#[tokio::test]
#[ignore]
async fn a_key_scoped_to_view_invoices_can_get_invoice_status() {
    let Some(pg) = service().await else {
        return;
    };
    let (owner, invoice, _payment) = seed_store_invoice_payment(&pg).await;
    let state = app_state(Arc::new(pg));

    let result = get_invoice_status(
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
        "a key scoped to canviewinvoices must be able to read invoice status: {:?}",
        result.err()
    );
}

// ---------------------------------------------------------------------------
// lookup_by_tx_hash
// ---------------------------------------------------------------------------

fn tx_hash_path(chain_id: &ChainId, tx_hash: &str) -> TxHashLookupPath {
    TxHashLookupPath {
        chain_id: chain_id.to_string(),
        tx_hash: tx_hash.to_string(),
    }
}

#[tokio::test]
#[ignore]
async fn a_key_scoped_to_create_invoice_is_refused_lookup_by_tx_hash() {
    let Some(pg) = service().await else {
        return;
    };
    let (owner, _invoice, payment) = seed_store_invoice_payment(&pg).await;
    let state = app_state(Arc::new(pg));

    let result = lookup_by_tx_hash(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_CREATE_INVOICE.to_string()]),
        ),
        State(state),
        Path(tx_hash_path(&payment.chain_id, &payment.tx_hash)),
    )
    .await;

    let Err((status, _)) = result else {
        panic!("a key scoped only to cancreateinvoice must not be able to look up the tx hash");
    };
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "expected the key's narrower scope to refuse the lookup"
    );
}

#[tokio::test]
#[ignore]
async fn a_key_scoped_to_view_invoices_can_lookup_by_tx_hash() {
    let Some(pg) = service().await else {
        return;
    };
    let (owner, _invoice, payment) = seed_store_invoice_payment(&pg).await;
    let state = app_state(Arc::new(pg));

    let result = lookup_by_tx_hash(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_VIEW_INVOICES.to_string()]),
        ),
        State(state),
        Path(tx_hash_path(&payment.chain_id, &payment.tx_hash)),
    )
    .await;

    assert!(
        result.is_ok(),
        "a key scoped to canviewinvoices must be able to look up the tx hash: {:?}",
        result.err().map(|(status, _)| status)
    );
}

#[tokio::test]
#[ignore]
async fn a_preexisting_unscoped_key_still_gets_payment() {
    let Some(pg) = service().await else {
        return;
    };
    let (owner, _invoice, payment) = seed_store_invoice_payment(&pg).await;
    let state = app_state(Arc::new(pg));

    let result = get_payment(
        StoreScopedUser(user_info(owner), None),
        State(state),
        Path(payment.id),
    )
    .await;

    assert!(
        result.is_ok(),
        "an unscoped key must read the payment exactly as before: {:?}",
        result.err()
    );
}
