#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Guards the Sentry performance-tracing setup used by the `ethpayserver`
//! binary: a transaction name must come from the matched route pattern, not
//! the raw request path, and must be stable across requests that only differ
//! by an id in the path. Without the `tower-axum-matched-path` feature wired
//! up, every distinct id mints its own transaction name - unbounded
//! cardinality and an unreadable performance page.
//!
//! Review finding, fixed: an earlier version of this test rebuilt a small
//! router of its own instead of the real one, so it could not catch a
//! regression in `server.rs`'s actual route table or `.layer()` ordering -
//! it only proved sentry-tower behaves as documented when wired the way
//! that file wired it. This version calls `server::api::router` - the exact
//! function `server.rs` calls to build the router it serves - against a real
//! Postgres connection, and layers it the same way `server.rs` does, so a
//! reordering of the two `.layer()` calls or a dropped
//! `tower-axum-matched-path` feature there fails this test instead of only
//! showing up later on a live dashboard.
//!
//! Needs `DATABASE_URL`; skips (does not fail) when it's unset, the same
//! convention the other ignored integration tests in this directory use.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use sentry::protocol::EnvelopeItem;
use tower::ServiceExt;

use auth::{AuthConfig, AuthService};
use data_service::PgDataService;
use rates::NoOpRateProvider;
use server::services::RedisEVMMonitor;
use server::state::PgAppState;

async fn service() -> Option<PgDataService> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .ok()?;
    Some(PgDataService::new(pool))
}

/// Same auth-service construction `server.rs::main` uses (`AuthService::with_config`
/// over the real data service, default `AuthConfig`), so `router<A>`'s `A` here is
/// the production type, not a stand-in.
fn app_state(data_service: Arc<PgDataService>) -> PgAppState<AuthService<PgDataService>> {
    let auth_service = Arc::new(AuthService::with_config(
        Arc::clone(&data_service),
        AuthConfig::default(),
    ));
    PgAppState::new(
        data_service,
        auth_service,
        None::<Arc<RedisEVMMonitor>>,
        Arc::new(NoOpRateProvider),
        Arc::new(server::services::email::NoopEmailSender),
    )
}

// A plain `#[test]`, not `#[tokio::test]`: `sentry::test::with_captured_envelopes_options`
// runs its closure synchronously and installing a second Tokio runtime inside
// a closure already driven by one panics ("Cannot start a runtime from
// within a runtime"). `bootstrap_rt` drives both the pre-capture setup and
// the closure's own `block_on`, sequentially, never nested.
#[test]
#[ignore]
fn transaction_name_is_the_route_pattern_not_the_request_uri() {
    let bootstrap_rt = tokio::runtime::Runtime::new().expect("build bootstrap runtime");
    let Some(pg) = bootstrap_rt.block_on(service()) else {
        return;
    };
    let state = app_state(Arc::new(pg));

    // The exact router-building call `server.rs` makes, layered the exact
    // same way (same two layers, same order) - so this test exercises the
    // real wiring, not a stand-in for it.
    let app = server::api::router(state, false, None, None, None)
        .layer(sentry::integrations::tower::SentryHttpLayer::new().enable_transaction())
        .layer(sentry::integrations::tower::NewSentryLayer::<axum::extract::Request>::new_from_top());

    let options = sentry::ClientOptions {
        // Sample everything: the property under test is the transaction's
        // *name*, not the sampling knob, and an unsampled transaction is
        // never turned into an envelope for the test transport to capture.
        traces_sample_rate: 1.0,
        ..Default::default()
    };

    let envelopes = sentry::test::with_captured_envelopes_options(
        || {
            bootstrap_rt.block_on(async {
                for invoice_id in [
                    "11111111-1111-1111-1111-111111111111",
                    "22222222-2222-2222-2222-222222222222",
                ] {
                    let request = Request::builder()
                        .uri(format!("/checkout/{invoice_id}"))
                        .body(Body::empty())
                        .expect("build request");
                    let response = app.clone().oneshot(request).await.expect("router call");
                    // Neither id was seeded, so the real handler 404s - that's
                    // expected, and confirms the request actually reached the
                    // real `get_checkout` handler through the real route
                    // table rather than hitting a fallback.
                    assert_eq!(response.status(), StatusCode::NOT_FOUND);
                }
            });
        },
        options,
    );

    let transaction_names: Vec<Option<String>> = envelopes
        .iter()
        .filter_map(|envelope| {
            envelope.items().find_map(|item| match item {
                EnvelopeItem::Transaction(transaction) => Some(transaction.name.clone()),
                _ => None,
            })
        })
        .collect();

    assert_eq!(
        transaction_names.len(),
        2,
        "expected one transaction envelope per request, got {transaction_names:?}"
    );
    for name in transaction_names {
        assert_eq!(
            name.as_deref(),
            Some("GET /checkout/{invoice_id}"),
            "transaction name must be the matched route pattern, not a per-id URI"
        );
    }
}
