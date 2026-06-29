//! Queued webhook job and retry scheduling.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::WebhookPayload;

/// A queued webhook job waiting for delivery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookJob {
    /// Unique job ID.
    pub id: Uuid,

    /// Webhook URL to deliver to.
    pub webhook_url: String,

    /// Secret for HMAC signing.
    pub webhook_secret: String,

    /// The payload to deliver.
    pub payload: WebhookPayload,

    /// Number of delivery attempts so far.
    pub attempts: u32,

    /// Maximum number of attempts.
    pub max_attempts: u32,

    /// When this job was created.
    pub created_at: DateTime<Utc>,

    /// When to attempt delivery (for delayed retries).
    pub scheduled_at: DateTime<Utc>,
}

impl WebhookJob {
    /// Stripe-like retry delays: 1m, 5m, 30m, 2h, 12h, 24h.
    /// Index by (attempts - 1), clamped to last entry.
    const RETRY_DELAYS_SECS: [u64; 6] = [60, 300, 1800, 7200, 43200, 86400];

    /// Create a new webhook job.
    pub fn new(webhook_url: String, webhook_secret: String, payload: WebhookPayload) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            webhook_url,
            webhook_secret,
            payload,
            attempts: 0,
            max_attempts: 7,
            created_at: now,
            scheduled_at: now,
        }
    }

    /// Calculate delay for next retry using a Stripe-like backoff schedule.
    ///
    /// Attempt 1: 1 minute
    /// Attempt 2: 5 minutes
    /// Attempt 3: 30 minutes
    /// Attempt 4: 2 hours
    /// Attempt 5: 12 hours
    /// Attempt 6: 24 hours
    /// Total retry window: ~38.6 hours
    pub fn retry_delay(&self) -> Duration {
        let idx = self.attempts.saturating_sub(1) as usize;
        Duration::from_secs(Self::RETRY_DELAYS_SECS[idx.min(Self::RETRY_DELAYS_SECS.len() - 1)])
    }

    /// Check if job has exceeded max attempts.
    pub fn is_exhausted(&self) -> bool {
        self.attempts >= self.max_attempts
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::services::webhook::WebhookEventType;

    fn test_payload() -> WebhookPayload {
        WebhookPayload {
            event_id: Uuid::new_v4(),
            event_type: WebhookEventType::PaymentDetected,
            timestamp: Utc::now(),
            invoice_id: "test-invoice".to_string(),
            store_id: Uuid::new_v4(),
            status: "processing".to_string(),
            amount: "1000".to_string(),
            amount_received: "1000".to_string(),
            asset_symbol: "ETH".to_string(),
            chain_id: 1,
            network: Some("ethereum".to_string()),
            payment: None,
        }
    }

    #[test]
    fn test_webhook_job_new() {
        let job = WebhookJob::new(
            "https://example.com/webhook".to_string(),
            "secret123".to_string(),
            test_payload(),
        );

        assert_eq!(job.attempts, 0);
        assert_eq!(job.max_attempts, 7);
        assert_eq!(job.webhook_url, "https://example.com/webhook");
        assert!(!job.is_exhausted());
    }

    #[test]
    fn test_webhook_job_retry_delay() {
        let mut job = WebhookJob::new(
            "https://example.com/webhook".to_string(),
            "secret123".to_string(),
            test_payload(),
        );

        // Attempt 1: 60s (1 minute)
        job.attempts = 1;
        assert_eq!(job.retry_delay(), Duration::from_secs(60));

        // Attempt 2: 300s (5 minutes)
        job.attempts = 2;
        assert_eq!(job.retry_delay(), Duration::from_secs(300));

        // Attempt 3: 1800s (30 minutes)
        job.attempts = 3;
        assert_eq!(job.retry_delay(), Duration::from_secs(1800));

        // Attempt 4: 7200s (2 hours)
        job.attempts = 4;
        assert_eq!(job.retry_delay(), Duration::from_secs(7200));

        // Attempt 5: 43200s (12 hours)
        job.attempts = 5;
        assert_eq!(job.retry_delay(), Duration::from_secs(43200));

        // Attempt 6: 86400s (24 hours)
        job.attempts = 6;
        assert_eq!(job.retry_delay(), Duration::from_secs(86400));
    }

    #[test]
    fn test_webhook_job_is_exhausted() {
        let mut job = WebhookJob::new(
            "https://example.com/webhook".to_string(),
            "secret123".to_string(),
            test_payload(),
        );

        for i in 0..7 {
            job.attempts = i;
            assert!(
                !job.is_exhausted(),
                "should not be exhausted at attempt {i}"
            );
        }
        job.attempts = 7;
        assert!(job.is_exhausted(), "should be exhausted at attempt 7");
    }

    #[test]
    fn test_webhook_permanent_failure_after_7_attempts() {
        let total_retry_secs: u64 = WebhookJob::RETRY_DELAYS_SECS.iter().sum();
        // 60 + 300 + 1800 + 7200 + 43200 + 86400 = 138960 seconds (~38.6 hours)
        assert_eq!(total_retry_secs, 138_960);
    }

    #[test]
    fn test_webhook_job_serialization() {
        let payload = WebhookPayload {
            event_id: Uuid::new_v4(),
            event_type: WebhookEventType::InvoiceExpired,
            timestamp: Utc::now(),
            invoice_id: "inv_456".to_string(),
            store_id: Uuid::new_v4(),
            status: "expired".to_string(),
            amount: "500".to_string(),
            amount_received: "0".to_string(),
            asset_symbol: "USDC".to_string(),
            chain_id: 137,
            network: Some("polygon".to_string()),
            payment: None,
        };

        let job = WebhookJob::new(
            "https://example.com/hook".to_string(),
            "secret".to_string(),
            payload,
        );

        // Should serialize to JSON
        let json = serde_json::to_string(&job).unwrap();
        assert!(json.contains("inv_456"));
        assert!(json.contains("https://example.com/hook"));

        // Should deserialize back
        let deserialized: WebhookJob = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.webhook_url, job.webhook_url);
        assert_eq!(deserialized.payload.invoice_id, job.payload.invoice_id);
    }
}
