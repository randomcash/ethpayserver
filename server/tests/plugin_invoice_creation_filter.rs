#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Review finding, fixed: the filter's own unit tests exercised
//! `run_invoice_creation_filters` in isolation, but nothing called the actual
//! `create_invoice` handler with a `Deny`-returning filter registered on
//! `AppState` and checked what a merchant would actually see. The behaviour
//! ("a filter refusing invoice creation actually blocks it, and the merchant
//! sees a reason naming the subscription") is about behavior observable
//! through the endpoint, not the filter runner alone - the status code, the
//! reason surfacing through `invoice_error` into the response body, and that
//! the filter runs *after* a real permission check has already passed rather
//! than being indistinguishable from an auth rejection.
//!
//! Calls the handler function directly against a real database rather than
//! through the router: `StoreScopedUser` and `State` are plain data the
//! extractors produce, so nothing about this assertion depends on routing or
//! middleware, only on `create_invoice`'s own body.
//!
//! Review finding, fixed: this comment used to claim the filter
//! "runs after a real permission check has already passed rather than being
//! indistinguishable from an auth rejection" without a test asserting it.
//! Both rejections actually share HTTP 403 - `permission_denies_before_the_filter_is_ever_consulted`
//! below is the ordering test, and it distinguishes them by the `error` code
//! in the body (`forbidden` vs `invoice_creation_blocked`), not by status.

use std::sync::Arc;

use async_trait::async_trait;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use sqlx::PgPool;
use uuid::Uuid;

use auth::{
    Result as AuthResult, Role, Session, SessionId, SessionService, Store, UserId, UserInfo,
};
use data_service::PgDataService;
use data_service::store_creation::StoreCreationWriter;
use rates::NoOpRateProvider;
use server::api::StoreScopedUser;
use server::api::invoices::{CreateInvoiceRequest, create_invoice};
use server::services::RedisEVMMonitor;
use server::services::plugins::{
    FilterVerdict, InvoiceCreationFilter, InvoiceCreationFilterRequest,
};
use server::state::PgAppState;

/// Not exercised: `create_invoice` never calls back into session management,
/// only reads the already-authenticated `UserInfo` this test constructs
/// directly.
struct UnusedSessionService;

#[async_trait]
impl SessionService for UnusedSessionService {
    async fn validate_session(&self, _session_id: SessionId) -> AuthResult<(UserInfo, Session)> {
        unimplemented!("not exercised by create_invoice")
    }
    async fn logout(&self, _session_id: SessionId) -> AuthResult<()> {
        unimplemented!("not exercised by create_invoice")
    }
    async fn logout_all(&self, _session_id: SessionId) -> AuthResult<()> {
        unimplemented!("not exercised by create_invoice")
    }
    async fn cleanup_stale_sessions(&self) -> AuthResult<u64> {
        unimplemented!("not exercised by create_invoice")
    }
}

struct AlwaysDeny;

#[async_trait]
impl InvoiceCreationFilter for AlwaysDeny {
    async fn filter_invoice_creation(
        &self,
        _request: InvoiceCreationFilterRequest,
    ) -> FilterVerdict {
        FilterVerdict::Deny {
            reason: "Your subscription lapsed; renew it to create new invoices.".to_string(),
        }
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

fn app_state(
    data_service: Arc<PgDataService>,
    filters: Vec<Arc<dyn InvoiceCreationFilter>>,
) -> PgAppState<UnusedSessionService> {
    app_state_billing(data_service, filters, None)
}

fn app_state_billing(
    data_service: Arc<PgDataService>,
    filters: Vec<Arc<dyn InvoiceCreationFilter>>,
    billing_store_id: Option<types::StoreId>,
) -> PgAppState<UnusedSessionService> {
    let mut state = PgAppState::new(
        data_service,
        Arc::new(UnusedSessionService),
        None::<Arc<RedisEVMMonitor>>,
        Arc::new(NoOpRateProvider),
        Arc::new(server::services::email::NoopEmailSender),
    );
    state.invoice_creation_filters = filters;
    state.billing_store_id = billing_store_id;
    state
}

/// Ticket test 2, at the endpoint the review said was unverified.
#[tokio::test]
#[ignore]
async fn a_denying_filter_blocks_the_real_endpoint_with_the_reason() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user, with the Owner role's cancreateinvoice permission");

    let state = app_state(Arc::new(pg), vec![Arc::new(AlwaysDeny)]);

    let result = create_invoice(
        StoreScopedUser(user_info(owner), None),
        State(state),
        Json(invoice_request(store.id.0)),
    )
    .await;

    let Err((status, Json(body))) = result else {
        panic!("a denying filter must refuse invoice creation");
    };
    assert_eq!(status, StatusCode::FORBIDDEN);
    let message = body["message"].as_str().expect("error body has a message");
    assert!(
        message.contains("subscription"),
        "the merchant must see why, got: {message}"
    );
}

/// Review finding, fixed: the module comment claimed the filter runs
/// after the permission check without a test pinning the order. A denying
/// filter is installed here too, so if the filter ran first - or the
/// permission check were skipped - this request would come back
/// `invoice_creation_blocked` instead. It must come back `forbidden`: a user
/// with no role on the store never reaches the filter at all.
#[tokio::test]
#[ignore]
async fn permission_denies_before_the_filter_is_ever_consulted() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let stranger = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");

    let state = app_state(Arc::new(pg), vec![Arc::new(AlwaysDeny)]);

    let result = create_invoice(
        StoreScopedUser(user_info(stranger), None),
        State(state),
        Json(invoice_request(store.id.0)),
    )
    .await;

    let Err((status, Json(body))) = result else {
        panic!("a user with no role on the store must be refused");
    };
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        body["error"], "forbidden",
        "must fail on the permission check, not the filter: got {body:?}"
    );
}

/// The negative case: with no filters installed, the same owner/store passes
/// the filter stage untouched and fails downstream instead (no payment
/// methods configured) - proving the filter wiring only intervenes when a
/// filter actually denies, not on every request.
#[tokio::test]
#[ignore]
async fn no_filters_reaches_past_the_filter_stage() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");

    let state = app_state(Arc::new(pg), Vec::new());

    let result = create_invoice(
        StoreScopedUser(user_info(owner), None),
        State(state),
        Json(invoice_request(store.id.0)),
    )
    .await;

    let Err((status, Json(body))) = result else {
        panic!("expected a downstream failure past the filter stage (no wallet configured)");
    };
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "no_payment_methods");
}

/// The deadlock the billing-store exemption exists to prevent.
///
/// A billing plugin refuses invoice creation for a merchant in arrears. The
/// invoice that *renews* a subscription is itself created on the instance's
/// own store - so without the exemption, a plugin that refuses (a bug, or
/// simply being down while its manifest fails closed) refuses the renewal
/// that would have cleared the refusal, and the only way out is editing the
/// database by hand.
///
/// Uses the same `AlwaysDeny` filter as the test above, which proves the
/// difference is the exemption and not the filter.
#[tokio::test]
#[ignore]
async fn our_own_billing_store_is_never_filtered() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");

    let state = app_state_billing(
        Arc::new(pg),
        vec![Arc::new(AlwaysDeny)],
        Some(types::StoreId(store.id.0)),
    );

    let result = create_invoice(
        StoreScopedUser(user_info(owner), None),
        State(state),
        Json(invoice_request(store.id.0)),
    )
    .await;

    if let Err((status, Json(body))) = &result {
        assert_ne!(
            body["error"].as_str(),
            Some("invoice_creation_blocked"),
            "the billing store was filtered; a plugin can now deadlock its own renewals (status {status})"
        );
    }
}

/// The exemption must be exactly one store wide. A second store on the same
/// instance is still filtered, or the exemption has become a way to bypass
/// billing entirely.
#[tokio::test]
#[ignore]
async fn the_exemption_covers_only_the_billing_store() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let billing = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    let merchant = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&billing, UserId(owner))
        .await
        .expect("seed billing store");
    pg.create_store_owned_by(&merchant, UserId(owner))
        .await
        .expect("seed merchant store");

    let state = app_state_billing(
        Arc::new(pg),
        vec![Arc::new(AlwaysDeny)],
        Some(types::StoreId(billing.id.0)),
    );

    let result = create_invoice(
        StoreScopedUser(user_info(owner), None),
        State(state),
        Json(invoice_request(merchant.id.0)),
    )
    .await;

    let Err((_, Json(body))) = result else {
        panic!("a merchant store must still be filtered when a billing store is configured");
    };
    assert_eq!(body["error"].as_str(), Some("invoice_creation_blocked"));
}

/// The account a filter is told about must be the store's **owner**, not the
/// caller and not a placeholder.
///
/// Billing is per merchant: a wrong account here bills or refuses the wrong
/// merchant, and every other test in this file passes whatever value is in
/// that field. This is the only one that reads it.
#[tokio::test]
#[ignore]
async fn the_filter_is_told_which_account_owns_the_store() {
    use std::sync::Mutex;

    struct Recording(Arc<Mutex<Vec<InvoiceCreationFilterRequest>>>);

    #[async_trait]
    impl InvoiceCreationFilter for Recording {
        async fn filter_invoice_creation(
            &self,
            request: InvoiceCreationFilterRequest,
        ) -> FilterVerdict {
            self.0.lock().unwrap().push(request);
            FilterVerdict::Allow
        }
    }

    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store");

    let seen = Arc::new(Mutex::new(Vec::new()));
    let state = app_state(Arc::new(pg), vec![Arc::new(Recording(seen.clone()))]);

    let _ = create_invoice(
        StoreScopedUser(user_info(owner), None),
        State(state),
        Json(invoice_request(store.id.0)),
    )
    .await;

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "the filter was never consulted");
    assert_eq!(
        seen[0].store_id.0, store.id.0,
        "the filter was told about the wrong store"
    );
    assert_eq!(
        seen[0].account_id,
        UserId(owner),
        "the filter was told about the wrong account; billing would act on the wrong merchant"
    );
}
