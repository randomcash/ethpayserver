//! Idempotency key middleware for POST endpoints.
//!
//! Accepts an `Idempotency-Key` header on mutation endpoints.
//! On first request: processes normally, caches the response in Redis with 24h TTL.
//! On retry with same key + same body: returns cached response with
//! `Idempotency-Replayed: true` header.
//! On retry with same key + different body: returns 409 Conflict.
//! On concurrent in-flight request with same key: returns 425 Too Early.
//!
//! Mounted as a `from_fn_with_state` layer on the invoice router, so it
//! covers every POST route under `/invoices` (invoice creation, cancel,
//! refund). Requests without the `Idempotency-Key` header or with a
//! non-POST method pass through unchanged.
//!
//! # Environment Variables
//!
//! - `IDEMPOTENCY_TTL_SECS` - Cache TTL in seconds (default: 86400 = 24h)

mod cache;
mod config;
mod headers;
mod key;
mod middleware;
mod response;

pub use middleware::middleware;

use config::DEFAULT_TTL_SECS;

/// Idempotency middleware state.
pub struct IdempotencyState {
    /// Shared Redis connection (cheap to clone).
    conn: redis::aio::MultiplexedConnection,
    /// Cache TTL in seconds.
    pub ttl_secs: u64,
}

impl IdempotencyState {
    /// Create from a Redis URL and environment config.
    pub async fn from_env(redis_url: &str) -> Result<Self, redis::RedisError> {
        let client = redis::Client::open(redis_url)?;
        let conn = client.get_multiplexed_async_connection().await?;
        let ttl_secs = std::env::var("IDEMPOTENCY_TTL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_TTL_SECS);
        Ok(Self { conn, ttl_secs })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use axum::body::Body;
    use axum::extract::Request;
    use axum::http::{HeaderValue, StatusCode};

    use super::cache::CachedResponse;
    use super::config::DEFAULT_TTL_SECS;
    use super::headers::is_replayable_header;
    use super::key::{compute_hash, extract_scope, validate_key};
    use super::response::build_replay_response;

    // ========================================================================
    // Key validation
    // ========================================================================

    #[test]
    fn validate_key_happy_path() {
        let val = HeaderValue::from_static("abc-123-def");
        assert!(validate_key(&val).is_some());
    }

    #[test]
    fn validate_key_uuid() {
        let val = HeaderValue::from_static("550e8400-e29b-41d4-a716-446655440000");
        assert!(validate_key(&val).is_some());
    }

    #[test]
    fn validate_key_empty_rejected() {
        let val = HeaderValue::from_static("");
        assert!(validate_key(&val).is_none());
    }

    #[test]
    fn validate_key_too_long_rejected() {
        let long = "a".repeat(256);
        let val = HeaderValue::from_str(&long).unwrap();
        assert!(validate_key(&val).is_none());
    }

    #[test]
    fn validate_key_max_length_accepted() {
        let max = "a".repeat(255);
        let val = HeaderValue::from_str(&max).unwrap();
        assert!(validate_key(&val).is_some());
    }

    // ========================================================================
    // Hash computation
    // ========================================================================

    #[test]
    fn compute_hash_deterministic() {
        let h1 = compute_hash("POST", "/invoices", b"body");
        let h2 = compute_hash("POST", "/invoices", b"body");
        assert_eq!(h1, h2);
    }

    #[test]
    fn compute_hash_different_body() {
        let h1 = compute_hash("POST", "/invoices", b"body1");
        let h2 = compute_hash("POST", "/invoices", b"body2");
        assert_ne!(h1, h2);
    }

    #[test]
    fn compute_hash_different_method() {
        let h1 = compute_hash("POST", "/invoices", b"body");
        let h2 = compute_hash("PUT", "/invoices", b"body");
        assert_ne!(h1, h2);
    }

    #[test]
    fn compute_hash_different_path() {
        let h1 = compute_hash("POST", "/invoices", b"body");
        let h2 = compute_hash("POST", "/other", b"body");
        assert_ne!(h1, h2);
    }

    #[test]
    fn compute_hash_is_sha256_hex() {
        let h = compute_hash("POST", "/invoices", b"body");
        assert_eq!(h.len(), 64); // SHA-256 = 32 bytes = 64 hex chars
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // ========================================================================
    // Scope extraction
    // ========================================================================

    #[test]
    fn extract_scope_valid_bearer() {
        let req = Request::builder()
            .header(
                "authorization",
                "Bearer 550e8400-e29b-41d4-a716-446655440000",
            )
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            extract_scope(&req),
            Some("550e8400-e29b-41d4-a716-446655440000".to_string())
        );
    }

    #[test]
    fn extract_scope_missing_header() {
        let req = Request::builder().body(Body::empty()).unwrap();
        assert!(extract_scope(&req).is_none());
    }

    #[test]
    fn extract_scope_basic_auth_ignored() {
        let req = Request::builder()
            .header("authorization", "Basic abc")
            .body(Body::empty())
            .unwrap();
        assert!(extract_scope(&req).is_none());
    }

    #[test]
    fn extract_scope_short_token_rejected() {
        let req = Request::builder()
            .header("authorization", "Bearer short")
            .body(Body::empty())
            .unwrap();
        assert!(extract_scope(&req).is_none());
    }

    // ========================================================================
    // Replay response
    // ========================================================================

    #[test]
    fn build_replay_response_status_and_headers() {
        let cached = CachedResponse {
            request_hash: "abc".to_string(),
            status: 201,
            body: b"{}".to_vec(),
            headers: vec![
                ("content-type".to_string(), "application/json".to_string()),
                ("location".to_string(), "/invoices/abc".to_string()),
                ("x-request-id".to_string(), "req-123".to_string()),
            ],
        };
        let resp = build_replay_response(&cached);
        assert_eq!(resp.status(), StatusCode::CREATED);
        assert_eq!(resp.headers().get("idempotency-replayed").unwrap(), "true");
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        assert_eq!(resp.headers().get("location").unwrap(), "/invoices/abc");
        assert_eq!(resp.headers().get("x-request-id").unwrap(), "req-123");
    }

    #[test]
    fn build_replay_response_no_headers() {
        let cached = CachedResponse {
            request_hash: "abc".to_string(),
            status: 200,
            body: vec![],
            headers: vec![],
        };
        let resp = build_replay_response(&cached);
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers().get("content-type").is_none());
        assert_eq!(resp.headers().get("idempotency-replayed").unwrap(), "true");
    }

    #[test]
    fn is_replayable_header_denies_hop_by_hop() {
        for name in &[
            "date",
            "server",
            "connection",
            "transfer-encoding",
            "content-length",
            "keep-alive",
            "idempotency-replayed",
            "DATE",
            "Content-Length",
        ] {
            assert!(!is_replayable_header(name), "{name} should be denied");
        }
    }

    #[test]
    fn is_replayable_header_allows_typical_response_headers() {
        for name in &[
            "content-type",
            "location",
            "x-request-id",
            "cache-control",
            "etag",
        ] {
            assert!(is_replayable_header(name), "{name} should be allowed");
        }
    }

    // ========================================================================
    // TTL
    // ========================================================================

    #[test]
    fn default_ttl_is_24h() {
        assert_eq!(DEFAULT_TTL_SECS, 86400);
    }
}
