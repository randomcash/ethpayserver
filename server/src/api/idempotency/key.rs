//! Idempotency-key validation, request scoping and request hashing.

use axum::{extract::Request, http::HeaderValue};
use sha2::{Digest, Sha256};

use super::config::MAX_KEY_LENGTH;

/// Validate the idempotency key header value.
/// Returns `Some(key)` if valid, `None` if invalid.
pub(super) fn validate_key(val: &HeaderValue) -> Option<String> {
    let key_str = val.to_str().ok()?;
    if key_str.is_empty() || key_str.len() > MAX_KEY_LENGTH || !key_str.is_ascii() {
        return None;
    }
    Some(key_str.to_string())
}
/// Extract scope identifier from the Authorization header.
///
/// Uses the Bearer token (session ID) as the scope key. When API-key auth
/// is added in the future, this can be extended to also accept `X-API-Key`.
pub(super) fn extract_scope(req: &Request) -> Option<String> {
    let auth_header = req.headers().get("authorization")?;
    let auth_str = auth_header.to_str().ok()?;
    let token = auth_str.strip_prefix("Bearer ")?;
    if token.len() < 32 {
        return None;
    }
    Some(token.to_string())
}

/// Compute SHA-256 hash of `method:path:body`.
pub(super) fn compute_hash(method: &str, path: &str, body: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(method.as_bytes());
    hasher.update(b":");
    hasher.update(path.as_bytes());
    hasher.update(b":");
    hasher.update(body);
    hex::encode(hasher.finalize())
}
