//! Server configuration.

use std::env;

/// Server configuration loaded from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    /// Database connection URL.
    pub database_url: String,

    /// HTTP server host.
    pub host: String,

    /// HTTP server port.
    pub port: u16,

    /// Log level (trace, debug, info, warn, error).
    pub log_level: String,

    /// Enable Swagger UI at /swagger-ui.
    pub enable_swagger: bool,
}

impl Config {
    /// Load configuration from environment variables.
    ///
    /// Required:
    /// - `DATABASE_URL` - PostgreSQL connection string
    ///
    /// Optional:
    /// - `HOST` - Server host (default: 127.0.0.1)
    /// - `PORT` - Server port (default: 3000)
    /// - `LOG_LEVEL` - Log level (default: info)
    /// - `ENABLE_SWAGGER` - Enable Swagger UI (default: true)
    pub fn from_env() -> anyhow::Result<Self> {
        let database_url = env::var("DATABASE_URL")
            .map_err(|_| anyhow::anyhow!("DATABASE_URL environment variable is required"))?;

        let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port = env::var("PORT")
            .unwrap_or_else(|_| "3000".to_string())
            .parse()
            .map_err(|_| anyhow::anyhow!("PORT must be a valid number"))?;

        let log_level = env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());

        let enable_swagger = env::var("ENABLE_SWAGGER")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(true);

        Ok(Self {
            database_url,
            host,
            port,
            log_level,
            enable_swagger,
        })
    }

    /// Get the server bind address.
    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}
