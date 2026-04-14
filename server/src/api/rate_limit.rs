//! Per-tier, IP-keyed rate limiting middleware.
//!
//! Classifies requests into tiers (auth, write, read, ws, health) and applies
//! separate governor-backed rate limiters per tier per client IP.
//!
//! # Environment Variables
//!
//! - `RATE_LIMIT_AUTH`  - Auth endpoint limit, req/min (default: 5)
//! - `RATE_LIMIT_WRITE` - Write endpoint limit, req/min (default: 10)
//! - `RATE_LIMIT_READ`  - Read endpoint limit, req/min (default: 60)
//! - `RATE_LIMIT_WS`    - WebSocket upgrade limit, req/min (default: 5)

use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::Arc;

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use governor::{
    Quota, RateLimiter,
    clock::{Clock, DefaultClock},
    state::keyed::DefaultKeyedStateStore,
};

use crate::metrics;

/// IP-keyed rate limiter.
type KeyedLimiter = RateLimiter<IpAddr, DefaultKeyedStateStore<IpAddr>, DefaultClock>;

/// Rate limit configuration loaded from environment variables.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Max requests per minute for auth endpoints.
    pub auth_rpm: u32,
    /// Max requests per minute for write endpoints.
    pub write_rpm: u32,
    /// Max requests per minute for read endpoints.
    pub read_rpm: u32,
    /// Max WebSocket upgrades per minute.
    pub ws_rpm: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            auth_rpm: 5,
            write_rpm: 10,
            read_rpm: 60,
            ws_rpm: 5,
        }
    }
}

impl RateLimitConfig {
    /// Load from environment variables with defaults.
    pub fn from_env() -> Self {
        Self {
            auth_rpm: parse_env_u32("RATE_LIMIT_AUTH", 5),
            write_rpm: parse_env_u32("RATE_LIMIT_WRITE", 10),
            read_rpm: parse_env_u32("RATE_LIMIT_READ", 60),
            ws_rpm: parse_env_u32("RATE_LIMIT_WS", 5),
        }
    }
}

/// Per-tier rate limiters.
pub struct RateLimitState {
    auth: KeyedLimiter,
    write: KeyedLimiter,
    read: KeyedLimiter,
    ws: KeyedLimiter,
}

impl RateLimitState {
    /// Create limiters from configuration.
    pub fn from_config(config: &RateLimitConfig) -> Self {
        Self {
            auth: make_limiter(config.auth_rpm),
            write: make_limiter(config.write_rpm),
            read: make_limiter(config.read_rpm),
            ws: make_limiter(config.ws_rpm),
        }
    }
}

/// Request classification tier.
#[derive(Debug, Clone, Copy)]
enum Tier {
    Health,
    Auth,
    Write,
    Read,
    WebSocket,
}

impl Tier {
    fn label(self) -> &'static str {
        match self {
            Tier::Health => "health",
            Tier::Auth => "auth",
            Tier::Write => "write",
            Tier::Read => "read",
            Tier::WebSocket => "ws",
        }
    }
}

/// Classify a request into a rate limit tier.
fn classify(path: &str, method: &Method) -> Tier {
    if path.starts_with("/health")
        || path == "/metrics"
        || path.starts_with("/swagger")
        || path.starts_with("/api-docs")
    {
        Tier::Health
    } else if path.starts_with("/auth") {
        Tier::Auth
    } else if path == "/ws" {
        Tier::WebSocket
    } else if *method == Method::GET || *method == Method::HEAD || *method == Method::OPTIONS {
        Tier::Read
    } else {
        Tier::Write
    }
}

/// Extract client IP from proxy headers, falling back to peer address.
fn client_ip(req: &Request, fallback: IpAddr) -> IpAddr {
    if let Some(xff) = req.headers().get("x-forwarded-for") {
        if let Ok(s) = xff.to_str() {
            if let Some(first) = s.split(',').next() {
                if let Ok(ip) = first.trim().parse::<IpAddr>() {
                    return ip;
                }
            }
        }
    }
    if let Some(xri) = req.headers().get("x-real-ip") {
        if let Ok(s) = xri.to_str() {
            if let Ok(ip) = s.trim().parse::<IpAddr>() {
                return ip;
            }
        }
    }
    fallback
}

/// Rate limiting middleware.
///
/// Classifies each request by tier, applies the matching per-IP limiter,
/// and returns 429 with Retry-After header when the limit is exceeded.
pub async fn middleware(
    State(limiters): State<Arc<RateLimitState>>,
    req: Request,
    next: Next,
) -> Response {
    let tier = classify(req.uri().path(), req.method());

    let limiter = match tier {
        Tier::Health => return next.run(req).await,
        Tier::Auth => &limiters.auth,
        Tier::Write => &limiters.write,
        Tier::Read => &limiters.read,
        Tier::WebSocket => &limiters.ws,
    };

    let fallback_addr = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or(IpAddr::from([127, 0, 0, 1]));
    let ip = client_ip(&req, fallback_addr);

    match limiter.check_key(&ip) {
        Ok(_) => next.run(req).await,
        Err(not_until) => {
            metrics::record_rate_limited(tier.label());
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

fn make_limiter(rpm: u32) -> KeyedLimiter {
    let rpm = NonZeroU32::new(rpm).unwrap_or(NonZeroU32::MIN);
    RateLimiter::keyed(Quota::per_minute(rpm))
}

fn parse_env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_health_endpoints() {
        assert!(matches!(classify("/health", &Method::GET), Tier::Health));
        assert!(matches!(
            classify("/health/live", &Method::GET),
            Tier::Health
        ));
        assert!(matches!(classify("/metrics", &Method::GET), Tier::Health));
        assert!(matches!(
            classify("/swagger-ui", &Method::GET),
            Tier::Health
        ));
        assert!(matches!(
            classify("/api-docs/openapi.json", &Method::GET),
            Tier::Health
        ));
    }

    #[test]
    fn classify_auth_endpoints() {
        assert!(matches!(
            classify("/auth/register", &Method::POST),
            Tier::Auth
        ));
        assert!(matches!(
            classify("/auth/login/complete", &Method::POST),
            Tier::Auth
        ));
    }

    #[test]
    fn classify_websocket() {
        assert!(matches!(classify("/ws", &Method::GET), Tier::WebSocket));
    }

    #[test]
    fn classify_read_vs_write() {
        assert!(matches!(classify("/invoices", &Method::GET), Tier::Read));
        assert!(matches!(classify("/stores", &Method::GET), Tier::Read));
        assert!(matches!(classify("/invoices", &Method::POST), Tier::Write));
        assert!(matches!(classify("/stores/abc", &Method::PUT), Tier::Write));
        assert!(matches!(
            classify("/stores/abc", &Method::DELETE),
            Tier::Write
        ));
    }

    #[test]
    fn config_defaults() {
        let config = RateLimitConfig::default();
        assert_eq!(config.auth_rpm, 5);
        assert_eq!(config.write_rpm, 10);
        assert_eq!(config.read_rpm, 60);
        assert_eq!(config.ws_rpm, 5);
    }

    #[test]
    fn limiter_allows_within_quota() {
        let limiter = make_limiter(10);
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(limiter.check_key(&ip).is_ok());
    }

    #[test]
    fn limiter_rejects_burst_over_quota() {
        let limiter = make_limiter(2);
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(limiter.check_key(&ip).is_ok());
        assert!(limiter.check_key(&ip).is_ok());
        assert!(limiter.check_key(&ip).is_err());
    }

    #[test]
    fn different_ips_are_independent() {
        let limiter = make_limiter(1);
        let ip1: IpAddr = "10.0.0.1".parse().unwrap();
        let ip2: IpAddr = "10.0.0.2".parse().unwrap();
        assert!(limiter.check_key(&ip1).is_ok());
        assert!(limiter.check_key(&ip2).is_ok());
        assert!(limiter.check_key(&ip1).is_err());
        assert!(limiter.check_key(&ip2).is_err());
    }

    #[test]
    fn health_not_rate_limited_label() {
        assert_eq!(Tier::Health.label(), "health");
        assert_eq!(Tier::Auth.label(), "auth");
        assert_eq!(Tier::Write.label(), "write");
        assert_eq!(Tier::Read.label(), "read");
        assert_eq!(Tier::WebSocket.label(), "ws");
    }
}
