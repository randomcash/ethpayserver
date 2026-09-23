//! Configuration for the webhook service.

use std::time::Duration;

/// Configuration for the webhook service.
#[derive(Debug, Clone)]
pub struct WebhookConfig {
    /// Redis key for the webhook job queue.
    pub queue_key: String,

    /// HTTP request timeout.
    pub request_timeout: Duration,

    /// How often to poll the queue when idle.
    pub poll_interval: Duration,

    /// How long to wait for a Redis connection before giving up.
    ///
    /// Without a bound, a Redis that is unreachable rather than merely slow
    /// is indistinguishable from a wedged process: both read as "no output"
    /// until whatever timeout the OS or network happens to apply, which can
    /// be minutes. This turns that into a fast, named failure.
    pub connect_timeout: Duration,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            queue_key: "ethpayserver:webhooks".to_string(),
            request_timeout: Duration::from_secs(30),
            poll_interval: Duration::from_secs(5),
            connect_timeout: Duration::from_secs(5),
        }
    }
}

impl WebhookConfig {
    /// Load configuration from environment variables.
    ///
    /// - `WEBHOOK_QUEUE_KEY` - Redis queue key (default: "ethpayserver:webhooks")
    /// - `WEBHOOK_REQUEST_TIMEOUT_SECS` - HTTP request timeout (default: 30)
    /// - `WEBHOOK_POLL_INTERVAL_SECS` - Queue poll interval (default: 5)
    /// - `WEBHOOK_REDIS_CONNECT_TIMEOUT_SECS` - Redis connect timeout (default: 5)
    pub fn from_env() -> Self {
        Self {
            queue_key: std::env::var("WEBHOOK_QUEUE_KEY")
                .unwrap_or_else(|_| "ethpayserver:webhooks".to_string()),
            request_timeout: Duration::from_secs(
                std::env::var("WEBHOOK_REQUEST_TIMEOUT_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(30),
            ),
            poll_interval: Duration::from_secs(
                std::env::var("WEBHOOK_POLL_INTERVAL_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(5),
            ),
            connect_timeout: Duration::from_secs(
                std::env::var("WEBHOOK_REDIS_CONNECT_TIMEOUT_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(5),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_webhook_config_default() {
        let config = WebhookConfig::default();
        assert_eq!(config.queue_key, "ethpayserver:webhooks");
        assert_eq!(config.request_timeout, Duration::from_secs(30));
        assert_eq!(config.poll_interval, Duration::from_secs(5));
        assert_eq!(config.connect_timeout, Duration::from_secs(5));
    }
}
