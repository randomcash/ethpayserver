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
