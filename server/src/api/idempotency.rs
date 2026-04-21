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

/// Maximum request body size to buffer (1 MiB).
///
/// Invoice create payloads are small (hundreds of bytes), so 1 MiB is a
/// generous ceiling while keeping the pre-handler DoS surface bounded —
/// callers sending larger bodies with an idempotency key get 413.
const MAX_REQUEST_BODY: usize = 1024 * 1024;

/// Maximum response body size to cache (1 MiB).
///
/// Well above expected JSON response size. Larger responses bypass the
/// cache (logged) rather than eating Redis memory.
const MAX_RESPONSE_BODY: usize = 1024 * 1024;

/// Lock TTL for in-flight requests (5 minutes).
///
/// Must be longer than the worst-case handler latency (DB contention,
/// address derivation, chain RPC). If the lock expires before the handler
/// finishes, a duplicate retry would bypass idempotency — so err on the
/// side of holding the lock too long rather than too short. Stale locks
/// self-clear via the TTL once the original handler returns.
const LOCK_TTL_SECS: u64 = 300;

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
    /// Response headers as (name, value) pairs. Non-UTF8 values are dropped on cache.
    #[serde(default)]
    headers: Vec<(String, String)>,
}

/// Hop-by-hop and server-generated headers that must not be replayed from cache.
/// On replay we reconstruct these from the live response context.
const NON_REPLAYABLE_HEADERS: &[&str] = &[
    "date",
    "server",
    "connection",
    "transfer-encoding",
    "content-length",
    "keep-alive",
    "upgrade",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "idempotency-replayed",
];

fn is_replayable_header(name: &str) -> bool {
    !NON_REPLAYABLE_HEADERS
        .iter()
        .any(|d| name.eq_ignore_ascii_case(d))
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
    if req.method() != Method::POST {
        return next.run(req).await;
    }
    let Some(key_header) = req.headers().get("idempotency-key").cloned() else {
        return next.run(req).await;
    };
    let Some(key_str) = validate_key(&key_header) else {
        return invalid_key_response();
    };
    // Missing auth — let the handler's auth extractor reject the request.
    let Some(scope) = extract_scope(&req) else {
        return next.run(req).await;
    };

    let (parts, body) = req.into_parts();
    let body_bytes = match axum::body::to_bytes(body, MAX_REQUEST_BODY).await {
        Ok(b) => b,
        Err(_) => {
            return (StatusCode::PAYLOAD_TOO_LARGE, "Request body too large").into_response();
        }
    };
    let request_hash = compute_hash(parts.method.as_str(), parts.uri.path(), &body_bytes);

    let cache_key = format!("{CACHE_PREFIX}:{scope}:{key_str}");
    let lock_key = format!("{LOCK_PREFIX}:{scope}:{key_str}");
    let mut conn = state.conn.clone();

    match cache_lookup(&mut conn, &cache_key, &request_hash).await {
        CacheLookup::Replay(resp) => return resp,
        CacheLookup::Conflict => return conflict_response(),
        CacheLookup::Miss => {}
        CacheLookup::Error(e) => {
            tracing::warn!(error = %e, "Redis GET failed for idempotency cache, falling through");
            let req = Request::from_parts(parts, Body::from(body_bytes));
            return next.run(req).await;
        }
    }

    if !acquire_lock(&mut conn, &lock_key).await {
        return in_flight_response();
    }

    let req = Request::from_parts(parts, Body::from(body_bytes));
    handle_and_cache(
        conn,
        cache_key,
        lock_key,
        request_hash,
        state.ttl_secs,
        req,
        next,
    )
    .await
}

/// Outcome of a cache GET.
enum CacheLookup {
    /// Same key + same body → replay the cached response.
    Replay(Response),
    /// Same key + different body → 409.
    Conflict,
    /// No cached entry (or entry was malformed and treated as miss).
    Miss,
    /// Redis unreachable — caller should fall through to the live handler.
    Error(redis::RedisError),
}

/// Look up a cached response and classify it against the current request hash.
async fn cache_lookup(
    conn: &mut redis::aio::MultiplexedConnection,
    cache_key: &str,
    request_hash: &str,
) -> CacheLookup {
    let cached: Option<Vec<u8>> = match redis::cmd("GET").arg(cache_key).query_async(conn).await {
        Ok(v) => v,
        Err(e) => return CacheLookup::Error(e),
    };
    let Some(bytes) = cached else {
        return CacheLookup::Miss;
    };
    let Ok(entry) = serde_json::from_slice::<CachedResponse>(&bytes) else {
        // Malformed cache entry — treat as miss so the handler runs fresh.
        return CacheLookup::Miss;
    };
    if entry.request_hash == request_hash {
        CacheLookup::Replay(build_replay_response(&entry))
    } else {
        CacheLookup::Conflict
    }
}

/// Try to acquire the in-flight lock via `SET NX EX`. Redis errors do not
/// block the request — idempotency is best-effort when Redis is unavailable.
async fn acquire_lock(conn: &mut redis::aio::MultiplexedConnection, lock_key: &str) -> bool {
    match redis::cmd("SET")
        .arg(lock_key)
        .arg("1")
        .arg("NX")
        .arg("EX")
        .arg(LOCK_TTL_SECS)
        .query_async::<Option<String>>(conn)
        .await
    {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(e) => {
            tracing::warn!(error = %e, "Redis SET NX failed for idempotency lock, falling through");
            true
        }
    }
}

/// Release the in-flight lock. Best-effort — errors are ignored because the
/// lock will self-expire via TTL.
async fn release_lock(conn: &mut redis::aio::MultiplexedConnection, lock_key: &str) {
    let _: Result<(), _> = redis::cmd("DEL").arg(lock_key).query_async(conn).await;
}

/// Run the downstream handler, then cache the response if it's a 2xx.
/// In all exit paths the in-flight lock is released.
async fn handle_and_cache(
    mut conn: redis::aio::MultiplexedConnection,
    cache_key: String,
    lock_key: String,
    request_hash: String,
    ttl_secs: u64,
    req: Request,
    next: Next,
) -> Response {
    let response = next.run(req).await;

    if !response.status().is_success() {
        release_lock(&mut conn, &lock_key).await;
        return response;
    }

    let (resp_parts, resp_body) = response.into_parts();
    let resp_bytes = match axum::body::to_bytes(resp_body, MAX_RESPONSE_BODY).await {
        Ok(b) => b,
        Err(_) => {
            release_lock(&mut conn, &lock_key).await;
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let entry = CachedResponse {
        request_hash,
        status: resp_parts.status.as_u16(),
        body: resp_bytes.to_vec(),
        headers: collect_cacheable_headers(&resp_parts.headers),
    };

    if let Ok(json) = serde_json::to_vec(&entry) {
        let _: Result<(), _> = redis::cmd("SETEX")
            .arg(&cache_key)
            .arg(ttl_secs)
            .arg(&json)
            .query_async(&mut conn)
            .await;
    }
    release_lock(&mut conn, &lock_key).await;

    Response::from_parts(resp_parts, Body::from(resp_bytes))
}

/// Filter response headers to only the ones safe to replay, discarding
/// non-UTF8 values.
fn collect_cacheable_headers(headers: &axum::http::HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, val)| {
            let name_str = name.as_str();
            if !is_replayable_header(name_str) {
                return None;
            }
            val.to_str()
                .ok()
                .map(|v| (name_str.to_string(), v.to_string()))
        })
        .collect()
}

/// 409 response for an idempotency key reused with a different body.
fn conflict_response() -> Response {
    (
        StatusCode::CONFLICT,
        axum::Json(serde_json::json!({"error": "idempotency_key_reuse"})),
    )
        .into_response()
}

/// 425 Too Early response when a concurrent request with the same key is
/// still in-flight.
fn in_flight_response() -> Response {
    (
        #[allow(clippy::expect_used)] // 425 is always a valid status code
        StatusCode::from_u16(425).expect("425 is a valid status code"),
        axum::Json(serde_json::json!({"error": "idempotency_in_progress"})),
    )
        .into_response()
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
///
/// Restores every cached header (content-type, location, pagination, custom
/// x-\* headers). Hop-by-hop and server-generated headers are excluded at
/// cache time (see `NON_REPLAYABLE_HEADERS`), so the stored set is safe to
/// replay verbatim.
fn build_replay_response(cached: &CachedResponse) -> Response {
    let status = StatusCode::from_u16(cached.status).unwrap_or(StatusCode::OK);
    let mut builder = Response::builder().status(status);

    for (name, value) in &cached.headers {
        builder = builder.header(name, value);
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
