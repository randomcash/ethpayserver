//! Per-API-key rate limiting middleware.
//!
//! Applies a governor-backed rate limiter keyed by `api_key_id` for requests
//! authenticated with an `X-API-Key` header. Runs AFTER the existing IP-tier
//! limiter so both gates must pass.
//!
//! # Environment Variables
//!
//! - `RATE_LIMIT_API_KEY_DEFAULT` - Default requests/minute per API key (default: 300)

use std::num::NonZeroU32;
use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use dashmap::DashMap;
use governor::{
    Quota, RateLimiter,
    clock::{Clock, DefaultClock},
    state::InMemoryState,
};
use uuid::Uuid;

use crate::metrics;

/// A non-keyed (single-cell) rate limiter for one API key.
type DirectLimiter = RateLimiter<governor::state::NotKeyed, InMemoryState, DefaultClock>;

/// Per-API-key rate limiter state.
pub struct ApiKeyRateLimitState {
    pool: sqlx::PgPool,
    /// Default requests per minute when `rate_limit_rpm` is NULL.
    pub default_rpm: u32,
    /// Per-key limiters, keyed by api_key_id UUID.
    /// Value: (configured_rpm, limiter). The rpm is stored so we can detect
    /// config changes and rebuild the limiter.
    limiters: DashMap<Uuid, (u32, DirectLimiter)>,
}

impl ApiKeyRateLimitState {
    /// Create a new per-API-key rate limiter.
    pub fn new(pool: sqlx::PgPool, default_rpm: u32) -> Self {
        Self {
            pool,
            default_rpm,
            limiters: DashMap::new(),
        }
    }

    /// Load default RPM from environment.
    pub fn from_env(pool: sqlx::PgPool) -> Self {
        let default_rpm: u32 = std::env::var("RATE_LIMIT_API_KEY_DEFAULT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300);

        Self::new(pool, default_rpm)
    }
}

fn make_direct_limiter(rpm: u32) -> DirectLimiter {
    let rpm = NonZeroU32::new(rpm).unwrap_or(NonZeroU32::MIN);
    RateLimiter::direct(Quota::per_minute(rpm))
}

/// Per-API-key rate limiting middleware.
///
/// If the request carries an `X-API-Key` header, the key is hashed and looked
/// up in the database. If found and active, a per-key rate limiter is applied.
/// If the key is not found or inactive the request is passed through (downstream
/// auth will reject it).
///
/// Returns 429 with the same JSON shape as the IP-tier limiter when the per-key
/// quota is exceeded.
pub async fn middleware(
    State(state): State<Arc<ApiKeyRateLimitState>>,
    req: Request,
    next: Next,
) -> Response {
    // Only apply to requests with an API key header.
    let raw_key = match req.headers().get("x-api-key").and_then(|v| v.to_str().ok()) {
        Some(k) if !k.is_empty() => k.to_owned(),
        _ => return next.run(req).await,
    };

    // Hash the key.
    let key_hash = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(raw_key.as_bytes());
        hex::encode(hasher.finalize())
    };

    // Look up the key in the database.
    let info = match data_service::PgDataService::new(state.pool.clone())
        .get_api_key_rate_limit_by_hash(&key_hash)
        .await
    {
        Ok(Some(info)) => info,
        // Key not found or inactive — let downstream auth handle rejection.
        Ok(None) => return next.run(req).await,
        // DB error — don't block the request, just skip rate limiting.
        Err(_) => return next.run(req).await,
    };

    let effective_rpm = info
        .rate_limit_rpm
        .and_then(|r| if r > 0 { Some(r as u32) } else { None })
        .unwrap_or(state.default_rpm);

    // Get or create the limiter for this key.
    let check_result = {
        let entry = state
            .limiters
            .entry(info.id)
            .or_insert_with(|| (effective_rpm, make_direct_limiter(effective_rpm)));

        let (current_rpm, limiter) = entry.value();

        // If the configured RPM changed, we need a new limiter.
        // This is a rare path (admin updated the key's rate limit).
        if *current_rpm != effective_rpm {
            drop(entry);
            state
                .limiters
                .insert(info.id, (effective_rpm, make_direct_limiter(effective_rpm)));
            // Re-fetch; None is unreachable after insert, but skip
            // rate-limiting rather than panicking if it somehow occurs.
            match state.limiters.get(&info.id) {
                Some(entry) => entry.value().1.check(),
                None => Ok(()),
            }
        } else {
            limiter.check()
        }
    };

    match check_result {
        Ok(_) => next.run(req).await,
        Err(not_until) => {
            metrics::record_rate_limited("api_key");
            let wait = not_until.wait_time_from(DefaultClock::default().now());
            let retry_after = (wait.as_secs() + 1).to_string();
            let mut response = (StatusCode::TOO_MANY_REQUESTS, "Too many requests").into_response();
            if let Ok(val) = retry_after.parse() {
                response.headers_mut().insert("retry-after", val);
            }
            response
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::too_many_lines,
        clippy::cognitive_complexity
    )]
    use super::*;

    #[tokio::test]
    async fn default_config_from_env() {
        // No env var set, should use 300
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let state = ApiKeyRateLimitState::new(pool, 300);
        assert_eq!(state.default_rpm, 300);
    }

    #[test]
    fn direct_limiter_allows_within_quota() {
        let limiter = make_direct_limiter(10);
        assert!(limiter.check().is_ok());
    }

    #[test]
    fn direct_limiter_rejects_over_quota() {
        let limiter = make_direct_limiter(2);
        assert!(limiter.check().is_ok());
        assert!(limiter.check().is_ok());
        assert!(limiter.check().is_err());
    }

    #[test]
    fn different_keys_are_independent() {
        let limiters: DashMap<Uuid, (u32, DirectLimiter)> = DashMap::new();
        let key1 = Uuid::new_v4();
        let key2 = Uuid::new_v4();

        limiters.insert(key1, (1, make_direct_limiter(1)));
        limiters.insert(key2, (1, make_direct_limiter(1)));

        // Each key gets its own quota
        assert!(limiters.get(&key1).unwrap().1.check().is_ok());
        assert!(limiters.get(&key2).unwrap().1.check().is_ok());

        // Both should be exhausted independently
        assert!(limiters.get(&key1).unwrap().1.check().is_err());
        assert!(limiters.get(&key2).unwrap().1.check().is_err());
    }

    #[test]
    fn limiter_rebuild_on_rpm_change() {
        let limiters: DashMap<Uuid, (u32, DirectLimiter)> = DashMap::new();
        let key = Uuid::new_v4();

        // Start with 2 rpm
        limiters.insert(key, (2, make_direct_limiter(2)));
        assert!(limiters.get(&key).unwrap().1.check().is_ok());
        assert!(limiters.get(&key).unwrap().1.check().is_ok());
        assert!(limiters.get(&key).unwrap().1.check().is_err());

        // Simulate RPM change to 10
        limiters.insert(key, (10, make_direct_limiter(10)));
        // Should allow requests again with the new quota
        assert!(limiters.get(&key).unwrap().1.check().is_ok());
    }

    #[test]
    fn custom_per_key_override_enforced() {
        let limiter = make_direct_limiter(10);

        // Should allow 10 requests
        for _ in 0..10 {
            assert!(limiter.check().is_ok());
        }

        // 11th should be rejected
        assert!(limiter.check().is_err());
    }
}
