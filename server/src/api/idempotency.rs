//! Idempotency key middleware for POST endpoints.
//!
//! Accepts an `Idempotency-Key` header on mutation endpoints.
//! On first request: processes normally, caches the response in Redis with 24h TTL.
//! On retry with same key + same body: returns cached response with
//! `Idempotency-Replayed: true` header.
//! On retry with same key + different body: returns 409 Conflict.
//! On concurrent in-flight request with same key: returns 425 Too Early.
//!
//! Applied only to `POST /v1/invoices` in this version; the layer is reusable
//! for future POST endpoints (`/v1/refunds`, `/v1/payouts`, etc.).
//!
//! # Environment Variables
//!
//! - `IDEMPOTENCY_TTL_SECS` - Cache TTL in seconds (default: 86400 = 24h)

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Maximum length for idempotency key values.
const MAX_KEY_LENGTH: usize = 255;

/// Maximum request body size to buffer (16 MiB).
const MAX_REQUEST_BODY: usize = 16 * 1024 * 1024;

/// Maximum response body size to cache (16 MiB).
const MAX_RESPONSE_BODY: usize = 16 * 1024 * 1024;

/// Lock TTL for in-flight requests (60 seconds).
const LOCK_TTL_SECS: u64 = 60;

/// Redis key prefix for idempotency cache entries.
const CACHE_PREFIX: &str = "idem";

/// Redis key prefix for in-flight locks.
const LOCK_PREFIX: &str = "idem_lock";

/// Default cache TTL: 24 hours.
const DEFAULT_TTL_SECS: u64 = 86400;

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

/// Cached response stored in Redis.
#[derive(Serialize, Deserialize)]
struct CachedResponse {
    /// SHA-256 hash of the original request (method + path + body).
    request_hash: String,
    /// HTTP status code.
    status: u16,
    /// Response body bytes.
    body: Vec<u8>,
    /// Content-Type header value.
    content_type: Option<String>,
}

/// Idempotency middleware.
///
/// Only activates when the `Idempotency-Key` header is present on a POST request.
/// Passes through all other requests unchanged.
pub async fn middleware(
    State(state): State<Arc<IdempotencyState>>,
    req: Request,
    next: Next,
) -> Response {
    // Only process POST requests with Idempotency-Key header
    if req.method() != Method::POST {
        return next.run(req).await;
    }

    let key_header = match req.headers().get("idempotency-key") {
        Some(val) => val.clone(),
        None => return next.run(req).await,
    };

    // Validate key
    let key_str = match validate_key(&key_header) {
        Some(k) => k,
        None => return invalid_key_response(),
    };

    // Extract auth scope from Authorization header (session ID)
    let scope = match extract_scope(&req) {
        Some(s) => s,
        // No auth header — let the handler's auth extractor reject the request
        None => return next.run(req).await,
    };

    // Buffer request body for hashing
    let (parts, body) = req.into_parts();
    let body_bytes = match axum::body::to_bytes(body, MAX_REQUEST_BODY).await {
        Ok(b) => b,
        Err(_) => {
            return (StatusCode::PAYLOAD_TOO_LARGE, "Request body too large").into_response();
        }
    };

    // Compute request hash: SHA-256(method:path:body)
    let request_hash = compute_hash(parts.method.as_str(), parts.uri.path(), &body_bytes);

    // Redis keys
    let cache_key = format!("{CACHE_PREFIX}:{scope}:{key_str}");
    let lock_key = format!("{LOCK_PREFIX}:{scope}:{key_str}");

    // Clone the shared connection (cheap — shares underlying TCP socket)
    let mut conn = state.conn.clone();

    // Check for cached response
    let cached: Option<Vec<u8>> = match redis::cmd("GET")
        .arg(&cache_key)
        .query_async(&mut conn)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "Redis GET failed for idempotency cache, falling through");
            let req = Request::from_parts(parts, Body::from(body_bytes));
            return next.run(req).await;
        }
    };

    if let Some(cached_bytes) = cached
        && let Ok(cached) = serde_json::from_slice::<CachedResponse>(&cached_bytes)
    {
        if cached.request_hash == request_hash {
            // Same key + same body → replay cached response
            return build_replay_response(&cached);
        }
        // Same key + different body → conflict
        return (
            StatusCode::CONFLICT,
            axum::Json(serde_json::json!({"error": "idempotency_key_reuse"})),
        )
            .into_response();
    }

    // Try to acquire in-flight lock (SET NX EX)
    let lock_acquired: bool = match redis::cmd("SET")
        .arg(&lock_key)
        .arg("1")
        .arg("NX")
        .arg("EX")
        .arg(LOCK_TTL_SECS)
        .query_async::<Option<String>>(&mut conn)
        .await
    {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(e) => {
            tracing::warn!(error = %e, "Redis SET NX failed for idempotency lock, falling through");
            true // Don't block on Redis errors
        }
    };

    if !lock_acquired {
        // Another request with the same key is in-flight
        return (
            #[allow(clippy::expect_used)] // 425 is always a valid status code
            StatusCode::from_u16(425).expect("425 is a valid status code"),
            axum::Json(serde_json::json!({"error": "idempotency_in_progress"})),
        )
            .into_response();
    }

    // Reconstruct request and call handler
    let req = Request::from_parts(parts, Body::from(body_bytes));
    let response = next.run(req).await;

    // Only cache 2xx responses; non-2xx lets the caller retry
    if !response.status().is_success() {
        let _: Result<(), _> = redis::cmd("DEL")
            .arg(&lock_key)
            .query_async(&mut conn)
            .await;
        return response;
    }

    // Buffer response body to cache it
    let (resp_parts, resp_body) = response.into_parts();
    let resp_bytes = match axum::body::to_bytes(resp_body, MAX_RESPONSE_BODY).await {
        Ok(b) => b,
        Err(_) => {
            let _: Result<(), _> = redis::cmd("DEL")
                .arg(&lock_key)
                .query_async(&mut conn)
                .await;
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let content_type = resp_parts
        .headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let entry = CachedResponse {
        request_hash,
        status: resp_parts.status.as_u16(),
        body: resp_bytes.to_vec(),
        content_type,
    };

    if let Ok(json) = serde_json::to_vec(&entry) {
        let _: Result<(), _> = redis::cmd("SETEX")
            .arg(&cache_key)
            .arg(state.ttl_secs)
            .arg(&json)
            .query_async(&mut conn)
            .await;
    }

    // Release the in-flight lock
    let _: Result<(), _> = redis::cmd("DEL")
        .arg(&lock_key)
        .query_async(&mut conn)
        .await;

    // Reconstruct response from buffered bytes
    Response::from_parts(resp_parts, Body::from(resp_bytes))
}

/// Validate the idempotency key header value.
/// Returns `Some(key)` if valid, `None` if invalid.
fn validate_key(val: &HeaderValue) -> Option<String> {
    let key_str = val.to_str().ok()?;
    if key_str.is_empty() || key_str.len() > MAX_KEY_LENGTH || !key_str.is_ascii() {
        return None;
    }
    Some(key_str.to_string())
}

/// 400 response for invalid idempotency keys.
fn invalid_key_response() -> Response {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(serde_json::json!({"error": "idempotency_key_invalid"})),
    )
        .into_response()
}

/// Extract scope identifier from the Authorization header.
///
/// Uses the Bearer token (session ID) as the scope key. When API-key auth
/// is added in the future, this can be extended to also accept `X-API-Key`.
fn extract_scope(req: &Request) -> Option<String> {
    let auth_header = req.headers().get("authorization")?;
    let auth_str = auth_header.to_str().ok()?;
    let token = auth_str.strip_prefix("Bearer ")?;
    if token.len() < 32 {
        return None;
    }
    Some(token.to_string())
}

/// Compute SHA-256 hash of `method:path:body`.
fn compute_hash(method: &str, path: &str, body: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(method.as_bytes());
    hasher.update(b":");
    hasher.update(path.as_bytes());
    hasher.update(b":");
    hasher.update(body);
    hex::encode(hasher.finalize())
}

/// Build a response from cached data with `Idempotency-Replayed: true` header.
fn build_replay_response(cached: &CachedResponse) -> Response {
    let status = StatusCode::from_u16(cached.status).unwrap_or(StatusCode::OK);
    let mut builder = Response::builder().status(status);

    if let Some(ref ct) = cached.content_type {
        builder = builder.header("content-type", ct);
    }
    builder = builder.header("idempotency-replayed", "true");

    builder
        .body(Body::from(cached.body.clone()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

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
            content_type: Some("application/json".to_string()),
        };
        let resp = build_replay_response(&cached);
        assert_eq!(resp.status(), StatusCode::CREATED);
        assert_eq!(resp.headers().get("idempotency-replayed").unwrap(), "true");
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
    }

    #[test]
    fn build_replay_response_no_content_type() {
        let cached = CachedResponse {
            request_hash: "abc".to_string(),
            status: 200,
            body: vec![],
            content_type: None,
        };
        let resp = build_replay_response(&cached);
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers().get("content-type").is_none());
        assert_eq!(resp.headers().get("idempotency-replayed").unwrap(), "true");
    }

    // ========================================================================
    // TTL
    // ========================================================================

    #[test]
    fn default_ttl_is_24h() {
        assert_eq!(DEFAULT_TTL_SECS, 86400);
    }
}
