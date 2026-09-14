//! Refunds are the merchant's job, not this server's.
//!
//! `create_refund` used to check an amount and write a `Pending` row that
//! nothing downstream would ever move — a promise the server could not keep,
//! since it holds no spending key. These tests pin the replacement: every
//! call gets the same explicit refusal, not a row that looks like progress.
//!
//! `calling_the_refund_endpoint_refuses_not_queues` goes through the same
//! `Router` wiring `api::mod` uses in production — same path, same
//! `create_refund::<A>` handler reference — rather than calling the private
//! `refund_unsupported()` helper directly. That matters here specifically:
//! a helper-only test would keep passing if `create_refund` were ever
//! changed to call something else first, add a conditional before the
//! refusal, or reintroduce a write path, because it would never touch the
//! function the router actually dispatches to.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test-only setup")]

use std::sync::Arc;

use async_trait::async_trait;
use auth::{DeviceId, Role, Session, SessionId, SessionService, UserId, UserInfo};
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    routing::post,
};
use chrono::Utc;
use data_service::PgDataService;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;
use uuid::Uuid;

use crate::state::{AppState, PgAppState};

use super::{REFUND_UNSUPPORTED_REASON, refund_unsupported};

// `ApiErr` is a private tuple struct defined in `crate::api`; this module is
// a descendant of it, so its fields are visible here even though
// `refunds.rs` itself only ever sees `ApiErr` as an opaque `IntoResponse`.
use crate::api::ApiErr;

/// `SessionService` double that authenticates any bearer session ID as one
/// fixed user. `create_refund` never reads `state.auth_service` for
/// anything but the `AuthenticatedUser` extractor's session check, so this
/// is the only piece of the auth service the test needs to supply.
struct AnySessionIsValid;

#[async_trait]
impl SessionService for AnySessionIsValid {
    async fn validate_session(&self, session_id: SessionId) -> auth::Result<(UserInfo, Session)> {
        let user_id = UserId(Uuid::new_v4());
        let user = UserInfo {
            id: user_id,
            email: None,
            primary_wallet_address: None,
            created_at: Utc::now(),
            last_login_at: None,
            role: Role::User,
        };
        let mut session = Session::new(user_id, DeviceId::new());
        session.id = session_id;
        Ok((user, session))
    }

    async fn logout(&self, _session_id: SessionId) -> auth::Result<()> {
        unimplemented!("create_refund never logs a session out")
    }

    async fn logout_all(&self, _session_id: SessionId) -> auth::Result<()> {
        unimplemented!("create_refund never logs a session out")
    }

    async fn cleanup_stale_sessions(&self) -> auth::Result<u64> {
        unimplemented!("create_refund never cleans up sessions")
    }
}

/// `RateProvider` double. `create_refund` never converts currency; this
/// only exists because `AppState` requires one.
struct NoRates;

#[async_trait]
impl rates::RateProvider for NoRates {
    async fn get_rate(
        &self,
        _from: &str,
        _to: &str,
    ) -> Result<rates::ExchangeRate, rates::RateError> {
        unimplemented!("create_refund never looks up a rate")
    }

    fn name(&self) -> &'static str {
        "none"
    }
}

/// A real `PgDataService` whose pool connects lazily and is never queried.
/// `create_refund` discards `State<PgAppState<A>>` entirely — that's the
/// behavior this test pins — so the pool never needs a live database behind
/// it. `connect_lazy` only fails on a malformed URL, never on an
/// unreachable host: the first real query is what would fail, and this test
/// asserts that no query ever happens.
fn never_queried_pg_data_service() -> PgDataService {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://refund-endpoint-test-unused/db")
        .expect("connect_lazy only validates the URL, it does not connect");
    PgDataService::new(pool)
}

fn test_state() -> PgAppState<AnySessionIsValid> {
    AppState::new(
        Arc::new(never_queried_pg_data_service()),
        Arc::new(AnySessionIsValid),
        None,
        Arc::new(NoRates),
    )
}

/// Mirrors the production mount in `api::mod` (same path shape, same
/// `create_refund::<A>` handler reference) so the request travels through
/// axum's extractors exactly as it would for a real caller.
#[tokio::test]
async fn calling_the_refund_endpoint_refuses_not_queues() {
    let app: Router = Router::new()
        .route(
            "/invoices/{invoice_id}/refund",
            post(super::create_refund::<AnySessionIsValid>),
        )
        .with_state(test_state());

    let req = Request::builder()
        .method("POST")
        .uri("/invoices/some-invoice-id/refund")
        .header("Authorization", format!("Bearer {}", Uuid::new_v4()))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::NOT_IMPLEMENTED,
        "a refund the server can never carry out must not look like an accepted one"
    );
    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    assert_eq!(body, REFUND_UNSUPPORTED_REASON.as_bytes());
}

#[test]
fn a_refund_request_is_refused_not_queued() {
    let ApiErr(status, reason) = refund_unsupported();

    assert_eq!(
        status,
        StatusCode::NOT_IMPLEMENTED,
        "a refund the server can never carry out must not look like an accepted one"
    );
    assert_eq!(reason, REFUND_UNSUPPORTED_REASON);
}

#[test]
fn the_refusal_names_the_reason() {
    // A bare status with no body reaches a caller as "HTTP error 501:" and
    // nothing else — the same dead end `ApiErr`'s own doc comment describes
    // for a reasonless 409. A merchant hitting this endpoint needs to learn
    // to refund from their own wallet, not guess why the request failed.
    assert!(!REFUND_UNSUPPORTED_REASON.is_empty());
}
