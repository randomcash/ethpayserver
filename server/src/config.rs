//! Server configuration.
//!
//! # Environment Variables
//!
//! ## Required
//! - `DATABASE_URL` - PostgreSQL connection string
//! - `REDIS_URL` - Redis connection URL for monitor communication
//!
//! ## Server
//! - `HOST` - Server host (default: 127.0.0.1)
//! - `PORT` - Server port (default: 3000)
//! - `LOG_LEVEL` - Log level: trace, debug, info, warn, error (default: info)
//! - `ENABLE_SWAGGER` - Enable Swagger UI at /swagger-ui (default: true)
//!
//! ## Redis Channels
//! - `REDIS_EVENTS_CHANNEL` - Redis channel for events (default: evmmonitor:events)
//! - `REDIS_COMMANDS_CHANNEL` - Redis channel for commands (default: evmmonitor:commands)
//!
//! ## Invoice Cleanup Service
//! - `CLEANUP_FALLBACK_INTERVAL_SECS` - Fallback check interval (default: 60)
//! - `CLEANUP_UNWATCH_GRACE_PERIOD_SECS` - Grace period before unwatching (default: 60)
//!
//! ## Webhook Service
//! - `WEBHOOK_QUEUE_KEY` - Redis queue key (default: ethpayserver:webhooks)
//! - `WEBHOOK_REQUEST_TIMEOUT_SECS` - HTTP request timeout (default: 30)
//! - `WEBHOOK_POLL_INTERVAL_SECS` - Queue poll interval (default: 5)
//!
//! ## CAPTCHA (optional)
//! - `CAPTCHA_PROVIDER` - Provider name: `turnstile` or `cloudflare` (unset = disabled)
//! - `CAPTCHA_SECRET_KEY` - Provider secret key (required when CAPTCHA_PROVIDER is set)
//! - `CAPTCHA_SITE_KEY` - Provider site key (required when CAPTCHA_PROVIDER is set)
//!
//! ## Rate Limiting
//! - `RATE_LIMIT_AUTH` - Auth endpoint limit, req/min per IP (default: 5)
//! - `RATE_LIMIT_WRITE` - Write endpoint limit, req/min per IP (default: 10)
//! - `RATE_LIMIT_READ` - Read endpoint limit, req/min per IP (default: 60)
//! - `RATE_LIMIT_WS` - WebSocket upgrade limit, req/min per IP (default: 5)
//!
//! ## Watch Retry Service
//! - `WATCH_RETRY_INTERVAL_SECS` - Retry interval in seconds (default: 30)
//! - `WATCH_RETRY_ENABLED` - Enable/disable retry service (default: true)

use std::env;

/// Server configuration loaded from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    /// Database connection URL.
    pub database_url: String,

    /// Redis connection URL for monitor communication.
    pub redis_url: Option<String>,

    /// HTTP server host.
    pub host: String,

    /// HTTP server port.
    pub port: u16,

    /// Log level (trace, debug, info, warn, error).
    pub log_level: String,

    /// Enable Swagger UI at /swagger-ui.
    pub enable_swagger: bool,
}

/// Valid log levels.
const VALID_LOG_LEVELS: &[&str] = &["trace", "debug", "info", "warn", "error"];

impl Config {
    /// Load configuration from environment variables.
    ///
    /// Required:
    /// - `DATABASE_URL` - PostgreSQL connection string
    ///
    /// Optional:
    /// - `REDIS_URL` - Redis connection URL for monitor communication
    /// - `HOST` - Server host (default: 127.0.0.1)
    /// - `PORT` - Server port (default: 3000)
    /// - `LOG_LEVEL` - Log level (default: info)
    /// - `ENABLE_SWAGGER` - Enable Swagger UI (default: true)
    pub fn from_env() -> anyhow::Result<Self> {
        let database_url = env::var("DATABASE_URL")
            .map_err(|_| anyhow::anyhow!("DATABASE_URL environment variable is required"))?;

        let redis_url = env::var("REDIS_URL").ok();

        let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port = env::var("PORT")
            .unwrap_or_else(|_| "3000".to_string())
            .parse()
            .map_err(|_| anyhow::anyhow!("PORT must be a valid number"))?;

        let log_level = env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());

        let enable_swagger = env::var("ENABLE_SWAGGER")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(true);

        let config = Self {
            database_url,
            redis_url,
            host,
            port,
            log_level,
            enable_swagger,
        };

        config.validate()?;
        Ok(config)
    }

    /// Validate configuration values.
    fn validate(&self) -> anyhow::Result<()> {
        // Validate DATABASE_URL format
        if !self.database_url.starts_with("postgres://")
            && !self.database_url.starts_with("postgresql://")
        {
            anyhow::bail!("DATABASE_URL must start with 'postgres://' or 'postgresql://'");
        }

        // Validate REDIS_URL format (if provided)
        if let Some(ref redis_url) = self.redis_url
            && !redis_url.starts_with("redis://")
            && !redis_url.starts_with("rediss://")
        {
            anyhow::bail!("REDIS_URL must start with 'redis://' or 'rediss://'");
        }

        // Validate LOG_LEVEL
        let log_level_lower = self.log_level.to_lowercase();
        if !VALID_LOG_LEVELS.contains(&log_level_lower.as_str()) {
            anyhow::bail!("LOG_LEVEL must be one of: {}", VALID_LOG_LEVELS.join(", "));
        }

        // Validate PORT range
        if self.port == 0 {
            anyhow::bail!("PORT must be between 1 and 65535");
        }

        Ok(())
    }

    /// Get the server bind address.
    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Derive the WebAuthn RP ID from the origin URL's host.
/// Returns `None` if the URL can't be parsed or has no host.
pub fn derive_rp_id_from_origin(origin: &str) -> Option<String> {
    url::Url::parse(origin)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
}

/// Validate that the WebAuthn RP ID is a registrable domain suffix of the origin host.
pub fn validate_rp_id(rp_id: &str, rp_origin: &str) -> bool {
    url::Url::parse(rp_origin)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
        .is_some_and(|host| host.ends_with(rp_id))
}

/// Parse the CAPTCHA provider from environment variables.
/// Returns `Ok(None)` if CAPTCHA is disabled (no CAPTCHA_PROVIDER set).
pub fn parse_captcha_env() -> anyhow::Result<Option<(String, String, String)>> {
    match env::var("CAPTCHA_PROVIDER").ok().as_deref() {
        Some(provider @ ("turnstile" | "cloudflare")) => {
            let secret = env::var("CAPTCHA_SECRET_KEY").map_err(|_| {
                anyhow::anyhow!("CAPTCHA_SECRET_KEY required when CAPTCHA_PROVIDER is set")
            })?;
            let site_key = env::var("CAPTCHA_SITE_KEY").map_err(|_| {
                anyhow::anyhow!("CAPTCHA_SITE_KEY required when CAPTCHA_PROVIDER is set")
            })?;
            Ok(Some((provider.to_string(), secret, site_key)))
        }
        Some(other) => {
            anyhow::bail!("Unknown CAPTCHA_PROVIDER: {other}. Supported: turnstile, cloudflare")
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    // ========================================================================
    // WebAuthn RP ID derivation
    // ========================================================================

    #[test]
    fn derive_rp_id_https() {
        assert_eq!(
            derive_rp_id_from_origin("https://app.example.com"),
            Some("app.example.com".to_string())
        );
    }

    #[test]
    fn derive_rp_id_with_port() {
        assert_eq!(
            derive_rp_id_from_origin("http://localhost:8080"),
            Some("localhost".to_string())
        );
    }

    #[test]
    fn derive_rp_id_ip() {
        assert_eq!(
            derive_rp_id_from_origin("http://192.168.1.1:3000"),
            Some("192.168.1.1".to_string())
        );
    }

    #[test]
    fn derive_rp_id_invalid_url() {
        assert_eq!(derive_rp_id_from_origin("not-a-url"), None);
    }

    #[test]
    fn derive_rp_id_empty() {
        assert_eq!(derive_rp_id_from_origin(""), None);
    }

    // ========================================================================
    // WebAuthn RP ID validation
    // ========================================================================

    #[test]
    fn validate_rp_id_exact_match() {
        assert!(validate_rp_id("example.com", "https://example.com"));
    }

    #[test]
    fn validate_rp_id_subdomain() {
        assert!(validate_rp_id("example.com", "https://app.example.com"));
    }

    #[test]
    fn validate_rp_id_mismatch() {
        assert!(!validate_rp_id("localhost", "https://app.example.com"));
    }

    #[test]
    fn validate_rp_id_localhost() {
        assert!(validate_rp_id("localhost", "http://localhost:8080"));
    }

    // ========================================================================
    // CAPTCHA config parsing
    // ========================================================================

    // SAFETY: these tests manipulate env vars which is unsafe in Rust 2024.
    // They must run serially (cargo test runs them in separate threads by default,
    // but env vars are process-global). Using --test-threads=1 for safety.

    #[test]
    fn captcha_disabled_when_no_env() {
        unsafe { env::remove_var("CAPTCHA_PROVIDER") };
        assert!(parse_captcha_env().unwrap().is_none());
    }

    #[test]
    fn captcha_turnstile_requires_keys() {
        unsafe {
            env::set_var("CAPTCHA_PROVIDER", "turnstile");
            env::remove_var("CAPTCHA_SECRET_KEY");
            env::remove_var("CAPTCHA_SITE_KEY");
        }

        let result = parse_captcha_env();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("CAPTCHA_SECRET_KEY")
        );

        unsafe { env::remove_var("CAPTCHA_PROVIDER") };
    }

    #[test]
    fn captcha_turnstile_with_keys() {
        unsafe {
            env::set_var("CAPTCHA_PROVIDER", "turnstile");
            env::set_var("CAPTCHA_SECRET_KEY", "secret123");
            env::set_var("CAPTCHA_SITE_KEY", "site123");
        }

        let result = parse_captcha_env().unwrap();
        assert!(result.is_some());
        let (provider, secret, site_key) = result.unwrap();
        assert_eq!(provider, "turnstile");
        assert_eq!(secret, "secret123");
        assert_eq!(site_key, "site123");

        unsafe {
            env::remove_var("CAPTCHA_PROVIDER");
            env::remove_var("CAPTCHA_SECRET_KEY");
            env::remove_var("CAPTCHA_SITE_KEY");
        }
    }

    #[test]
    fn captcha_cloudflare_alias() {
        unsafe {
            env::set_var("CAPTCHA_PROVIDER", "cloudflare");
            env::set_var("CAPTCHA_SECRET_KEY", "s");
            env::set_var("CAPTCHA_SITE_KEY", "k");
        }

        let result = parse_captcha_env().unwrap();
        assert_eq!(result.unwrap().0, "cloudflare");

        unsafe {
            env::remove_var("CAPTCHA_PROVIDER");
            env::remove_var("CAPTCHA_SECRET_KEY");
            env::remove_var("CAPTCHA_SITE_KEY");
        }
    }

    #[test]
    fn captcha_unknown_provider() {
        unsafe { env::set_var("CAPTCHA_PROVIDER", "recaptcha") };

        let result = parse_captcha_env();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unknown CAPTCHA_PROVIDER")
        );

        unsafe { env::remove_var("CAPTCHA_PROVIDER") };
    }
}
