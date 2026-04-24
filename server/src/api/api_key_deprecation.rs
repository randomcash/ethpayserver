//! Response-header middleware for the `X-API-Key-Deprecated` header.
//!
//! The auth extractor cannot set response headers directly; it returns
//! `UserInfo` and the handler owns the response type. Naively stashing
//! `ApiKeyDeprecationInfo` into request extensions from the extractor does
//! NOT work either — by the time the outer middleware gets the response
//! back from `next.run`, the request (and its extensions) have been
//! consumed, so any mutation the extractor made is unreachable.
//!
//! We bridge the gap with a shared slot:
//!
//!   1. `middleware` creates an `Arc<Mutex<Option<ApiKeyDeprecationInfo>>>`
//!      and inserts a clone into the request extensions BEFORE calling
//!      `next.run(req)`. It keeps a second clone of the Arc.
//!   2. Inside `next.run`, the auth extractor pulls the Arc out of
//!      extensions and writes the deprecation info into the Option.
//!   3. After `next.run` returns the Response, the outer middleware reads
//!      its retained Arc clone. If it contains Some, the middleware stamps
//!      `X-API-Key-Deprecated: rotate before <iso8601>` on the response.
//!
//! The Arc-shared-handle pattern is the standard axum workaround for
//! "extractor needs to talk to response middleware" because request
//! extensions don't survive `next.run`. Handler signatures and response
//! types are untouched.

use std::sync::{Arc, Mutex};

use axum::{
    extract::Request,
    http::{HeaderName, HeaderValue, header::HeaderMap},
    middleware::Next,
    response::Response,
};

use super::extractors::ApiKeyDeprecationInfo;

/// Shared slot type that the auth extractor writes into. The middleware
/// inserts the Arc into request extensions, then reads it back via its
/// retained clone after the handler runs.
pub type DeprecationSlot = Arc<Mutex<Option<ApiKeyDeprecationInfo>>>;

/// The response header name. Defined as a static so callers (tests, clients)
/// can reference the exact casing.
pub static X_API_KEY_DEPRECATED: HeaderName = HeaderName::from_static("x-api-key-deprecated");

/// Create a fresh empty slot and insert it into the request.
fn install_slot(req: &mut Request) -> DeprecationSlot {
    let slot: DeprecationSlot = Arc::new(Mutex::new(None));
    req.extensions_mut().insert(slot.clone());
    slot
}

/// Install the shared slot, run the handler, then stamp the response header
/// if the extractor populated the slot.
pub async fn middleware(mut req: Request, next: Next) -> Response {
    let slot = install_slot(&mut req);
    let mut response = next.run(req).await;
    // `.lock()` can only fail if a holder of the mutex panicked. Treat that
    // as "don't stamp the header" rather than propagating — we'd rather
    // serve the response than 500 it over a cosmetic header.
    if let Ok(mut guard) = slot.lock()
        && let Some(info) = guard.take()
    {
        add_deprecation_header(response.headers_mut(), &info);
    }
    response
}

fn add_deprecation_header(headers: &mut HeaderMap, info: &ApiKeyDeprecationInfo) {
    // `rotate before <iso8601>` — ISO-8601 is machine-parsable and
    // self-contained, no ambiguity across locales.
    let value = format!("rotate before {}", info.grace_deadline.to_rfc3339());
    if let Ok(hv) = HeaderValue::from_str(&value) {
        headers.insert(&X_API_KEY_DEPRECATED, hv);
    }
    // If the string was somehow not a valid header value (it always will be
    // for an ISO-8601 timestamp), we silently drop it rather than 500 the
    // request — the consumer still gets their response.
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test-only assertions"
)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::StatusCode, routing::get};
    use chrono::{Duration, Utc};
    use tower::util::ServiceExt;

    #[test]
    fn header_name_is_lowercase_canonical() {
        assert_eq!(X_API_KEY_DEPRECATED.as_str(), "x-api-key-deprecated");
    }

    #[test]
    fn adds_header_with_iso8601_deadline() {
        let mut headers = HeaderMap::new();
        let deprecated_at = Utc::now();
        let grace_deadline = deprecated_at + Duration::hours(48);
        let info = ApiKeyDeprecationInfo {
            deprecated_at,
            grace_deadline,
        };

        add_deprecation_header(&mut headers, &info);

        let value = headers
            .get(&X_API_KEY_DEPRECATED)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(value.starts_with("rotate before "));
        assert!(value.contains("T")); // ISO-8601 date/time separator
    }

    /// End-to-end: a handler that writes to the shared slot (simulating the
    /// auth extractor) causes the middleware to emit the header. This test
    /// is the reason this patch exists — the previous implementation stashed
    /// info into request extensions directly and the middleware could never
    /// read it back, so the header was never emitted.
    #[tokio::test]
    async fn middleware_emits_header_when_slot_is_populated_during_handler() {
        async fn handler_that_writes_slot(mut req: Request) -> &'static str {
            let slot = req
                .extensions_mut()
                .get::<DeprecationSlot>()
                .cloned()
                .expect("middleware must install slot before handler runs");
            let deprecated_at = Utc::now();
            *slot.lock().unwrap() = Some(ApiKeyDeprecationInfo {
                deprecated_at,
                grace_deadline: deprecated_at + Duration::hours(48),
            });
            "ok"
        }

        let app: Router = Router::new()
            .route("/", get(handler_that_writes_slot))
            .layer(axum::middleware::from_fn(middleware));

        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let hv = resp.headers().get(&X_API_KEY_DEPRECATED).unwrap();
        let value = hv.to_str().unwrap();
        assert!(
            value.starts_with("rotate before "),
            "header missing 'rotate before' prefix: {value}"
        );
    }

    /// Inverse: if nothing populated the slot, the header must NOT appear
    /// (else we'd be warning consumers every time a session-token request
    /// hit a route).
    #[tokio::test]
    async fn middleware_skips_header_when_slot_is_untouched() {
        async fn handler_noop() -> &'static str {
            "ok"
        }

        let app: Router = Router::new()
            .route("/", get(handler_noop))
            .layer(axum::middleware::from_fn(middleware));

        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            resp.headers().get(&X_API_KEY_DEPRECATED).is_none(),
            "header should not appear when the slot was never populated"
        );
    }
}
