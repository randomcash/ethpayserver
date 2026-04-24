//! Response-header middleware for the `X-API-Key-Deprecated` header.
//!
//! The auth extractor cannot set response headers directly; it returns
//! `UserInfo` and the handler owns the response type. To bridge that gap,
//! `validate_api_key` stamps an `ApiKeyDeprecationInfo` into request
//! extensions whenever a deprecated-but-still-valid key is used within its
//! grace window. This middleware reads that extension on the outgoing
//! response path and emits
//!
//!   `X-API-Key-Deprecated: rotate before <iso8601>`
//!
//! so API consumers learn their key needs rotating without changing the
//! handler signatures or response types.

use axum::{
    extract::Request,
    http::{HeaderName, HeaderValue, header::HeaderMap},
    middleware::Next,
    response::Response,
};

use super::extractors::ApiKeyDeprecationInfo;

/// The response header name. Defined as a static so callers (tests, clients)
/// can reference the exact casing.
pub static X_API_KEY_DEPRECATED: HeaderName = HeaderName::from_static("x-api-key-deprecated");

/// Read `ApiKeyDeprecationInfo` from the request extensions and, if present,
/// stamp the header on the response.
///
/// The header is emitted whether the handler succeeded or returned an error —
/// a deprecated key is still a deprecated key either way, and the consumer
/// needs the signal regardless of this particular request's outcome.
pub async fn middleware(req: Request, next: Next) -> Response {
    let deprecation = req.extensions().get::<ApiKeyDeprecationInfo>().cloned();
    let mut response = next.run(req).await;
    if let Some(info) = deprecation {
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
    // request — the consumer still gets their response and the server still
    // logs the deprecated key usage elsewhere.
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test-only assertions")]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

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
}
