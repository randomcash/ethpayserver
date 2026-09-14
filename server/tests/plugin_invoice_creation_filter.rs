#![allow(clippy::unwrap_used, clippy::expect_used)]

//! RCS-300 review finding, fixed: the filter's own unit tests exercised
//! `run_invoice_creation_filters` in isolation, but nothing called the actual
//! `create_invoice` handler with a `Deny`-returning filter registered on
//! `AppState` and checked what a merchant would actually see. Ticket test 2
//! ("a filter refusing invoice creation actually blocks it, and the merchant
//! sees a reason naming the subscription") is about behavior observable
//! through the endpoint, not the filter runner alone - the status code, the
//! reason surfacing through `invoice_error` into the response body, and that
//! the filter runs *after* a real permission check has already passed rather
//! than being indistinguishable from an auth rejection.
//!
//! Calls the handler function directly against a real database rather than
//! through the router: `AuthenticatedUser` and `State` are plain data the
//! extractors produce, so nothing about this assertion depends on routing or
//! middleware, only on `create_invoice`'s own body.

use std::sync::Arc;

use async_trait::async_trait;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use sqlx::PgPool;
use uuid::Uuid;

use auth::{Result as AuthResult, Role, Session, SessionId, SessionService, Store, UserId, UserInfo};
use data_service::PgDataService;
use data_service::store_creation::StoreCreationWriter;
use rates::NoOpRateProvider;
use server::api::AuthenticatedUser;
use server::api::invoices::{CreateInvoiceRequest, create_invoice};
use server::services::RedisEVMMonitor;
use server::services::plugins::{FilterVerdict, InvoiceCreationFilter, InvoiceCreationFilterRequest};
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
    let mut state = PgAppState::new(
        data_service,
        Arc::new(UnusedSessionService),
        None::<Arc<RedisEVMMonitor>>,
        Arc::new(NoOpRateProvider),
    );
    state.invoice_creation_filters = filters;
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
        AuthenticatedUser(user_info(owner)),
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
        AuthenticatedUser(user_info(owner)),
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
