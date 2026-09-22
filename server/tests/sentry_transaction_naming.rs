#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Guards the Sentry performance-tracing setup used by the `ethpayserver`
//! binary: a transaction name must come from the matched route pattern, not
//! the raw request path, and must be stable across requests that only differ
//! by an id in the path. Without the `tower-axum-matched-path` feature wired
//! up, every distinct id mints its own transaction name - unbounded
//! cardinality and an unreadable performance page. (Confirmed by temporarily
//! swapping that feature for the plain `tower-http` one: this test then fails
//! with the per-id URI as the transaction name, exactly the regression it
//! exists to catch.)
//!
//! This exercises the same two layers `server.rs` adds to its router, against
//! a capturing Sentry transport, so dropping the feature or swapping in a
//! router without a path-param route fails a test instead of only showing up
//! later on a live dashboard.

use axum::body::Body;
use axum::extract::Path;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use sentry::protocol::EnvelopeItem;
use tower::ServiceExt;

async fn get_invoice(Path(_id): Path<String>) -> StatusCode {
    StatusCode::OK
}

fn app() -> Router {
    Router::new()
        .route("/api/invoices/{id}", get(get_invoice))
        .layer(sentry::integrations::tower::SentryHttpLayer::new().enable_transaction())
        .layer(sentry::integrations::tower::NewSentryLayer::<axum::extract::Request>::new_from_top())
}

#[test]
fn transaction_name_is_the_route_pattern_not_the_request_uri() {
    let options = sentry::ClientOptions {
        // Sample everything: the property under test is the transaction's
        // *name*, not the sampling knob, and an unsampled transaction is
        // never turned into an envelope for the test transport to capture.
        traces_sample_rate: 1.0,
        ..Default::default()
    };

    let envelopes = sentry::test::with_captured_envelopes_options(
        || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build current-thread runtime");
            runtime.block_on(async {
                for id in [
                    "11111111-1111-1111-1111-111111111111",
                    "22222222-2222-2222-2222-222222222222",
                ] {
                    let request = Request::builder()
                        .uri(format!("/api/invoices/{id}"))
                        .body(Body::empty())
                        .expect("build request");
                    let response = app().oneshot(request).await.expect("router call");
                    assert_eq!(response.status(), StatusCode::OK);
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
            Some("GET /api/invoices/{id}"),
            "transaction name must be the matched route pattern, not a per-id URI"
        );
    }
}
