//! HTTP request metrics middleware.
//!
//! Records `ethpayserver_http_requests_total` counter and
//! `ethpayserver_http_request_duration_seconds` histogram per request.

use axum::{extract::Request, middleware::Next, response::Response};
use std::time::Instant;

use crate::metrics;

/// Middleware that records HTTP request count and latency.
pub async fn middleware(req: Request, next: Next) -> Response {
    let method = req.method().to_string();
    let path = normalize_path(req.uri().path());

    let start = Instant::now();
    let response = next.run(req).await;
    let duration = start.elapsed();

    let status = response.status().as_u16();
    metrics::record_http_request(&method, &path, status, duration);

    response
}

/// Normalize path to prevent high-cardinality labels.
///
/// Replaces UUID and numeric path segments with placeholders.
fn normalize_path(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if segment.is_empty() {
                return segment.to_string();
            }
            // Replace UUIDs (8-4-4-4-12 or bare 32+ hex chars)
            if segment.len() >= 32 && segment.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
                return ":id".to_string();
            }
            // Replace pure numeric segments
            if segment.chars().all(|c| c.is_ascii_digit()) {
                return ":id".to_string();
            }
            segment.to_string()
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn normalize_uuid_segments() {
        assert_eq!(
            normalize_path("/stores/550e8400-e29b-41d4-a716-446655440000"),
            "/stores/:id"
        );
        assert_eq!(
            normalize_path("/invoices/abc123def456/payments"),
            "/invoices/abc123def456/payments"
        );
    }

    #[test]
    fn normalize_numeric_segments() {
        assert_eq!(normalize_path("/evm/tokens/42"), "/evm/tokens/:id");
    }

    #[test]
    fn normalize_preserves_static_paths() {
        assert_eq!(normalize_path("/health/live"), "/health/live");
        assert_eq!(normalize_path("/auth/register"), "/auth/register");
        assert_eq!(normalize_path("/stores"), "/stores");
    }

    #[test]
    fn normalize_preserves_root() {
        assert_eq!(normalize_path("/"), "/");
    }
}
