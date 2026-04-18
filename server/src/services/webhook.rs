//! Webhook notification service.
//!
//! Delivers webhook notifications to merchants when invoice status changes.
//! Uses Redis as a job queue with exponential backoff retries.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use data_service::PaymentEventWriter;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::metrics;

/// Trait for data service requirements in WebhookService.
pub trait WebhookDataService: PaymentEventWriter + Send + Sync {}

impl<T> WebhookDataService for T where T: PaymentEventWriter + Send + Sync {}

/// Webhook event types that trigger notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookEventType {
    /// Payment detected (pending → processing)
    PaymentDetected,
    /// Payment confirmed (processing → paid)
    PaymentConfirmed,
    /// Invoice expired (pending → expired)
    InvoiceExpired,
    /// Invoice cancelled
    InvoiceCancelled,
    /// Late payment received (expired → late_paid)
    /// Requires merchant review before fulfillment.
    LatePaid,
    /// Refund transaction initiated.
    RefundInitiated,
    /// Refund transaction confirmed on-chain.
    RefundConfirmed,
    /// Refund transaction failed.
    RefundFailed,
    /// Payout/sweep transaction initiated.
    PayoutInitiated,
    /// Payout transaction confirmed on-chain.
    PayoutConfirmed,
    /// Payout transaction failed.
    PayoutFailed,
}

impl std::fmt::Display for WebhookEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PaymentDetected => write!(f, "payment_detected"),
            Self::PaymentConfirmed => write!(f, "payment_confirmed"),
            Self::InvoiceExpired => write!(f, "invoice_expired"),
            Self::InvoiceCancelled => write!(f, "invoice_cancelled"),
            Self::LatePaid => write!(f, "late_paid"),
            Self::RefundInitiated => write!(f, "refund_initiated"),
            Self::RefundConfirmed => write!(f, "refund_confirmed"),
            Self::RefundFailed => write!(f, "refund_failed"),
            Self::PayoutInitiated => write!(f, "payout_initiated"),
            Self::PayoutConfirmed => write!(f, "payout_confirmed"),
            Self::PayoutFailed => write!(f, "payout_failed"),
        }
    }
}

/// Webhook payload sent to merchant endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookPayload {
    /// Unique event ID for idempotency.
    pub event_id: Uuid,

    /// Event type.
    pub event_type: WebhookEventType,

    /// Timestamp when the event occurred.
    pub timestamp: DateTime<Utc>,

    /// Invoice ID.
    pub invoice_id: String,

    /// Store ID.
    pub store_id: Uuid,

    /// Current invoice status.
    pub status: String,

    /// Amount requested (in smallest unit).
    pub amount: String,

    /// Amount received so far (in smallest unit).
    pub amount_received: String,

    /// Asset symbol (e.g., "ETH", "USDT").
    pub asset_symbol: String,

    /// Chain ID (EIP-155).
    pub chain_id: u64,

    /// Network name (null for testnets/custom chains).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,

    /// Payment details (if applicable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment: Option<WebhookPaymentInfo>,
}

/// Payment information included in webhook payloads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookPaymentInfo {
    /// Transaction hash.
    pub tx_hash: String,

    /// Sender address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_address: Option<String>,

    /// Block number where payment was included.
    /// Confirmations can be computed as: current_block - block_number + 1
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_number: Option<u64>,

    /// Whether the payment has reached required confirmations.
    pub confirmed: bool,
}

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
    /// Create a new webhook job.
    pub fn new(webhook_url: String, webhook_secret: String, payload: WebhookPayload) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            webhook_url,
            webhook_secret,
            payload,
            attempts: 0,
            max_attempts: 3,
            created_at: now,
            scheduled_at: now,
        }
    }

    /// Calculate delay for next retry using exponential backoff.
    ///
    /// Attempt 1: 10 seconds
    /// Attempt 2: 30 seconds
    /// Attempt 3: 90 seconds (then give up)
    pub fn retry_delay(&self) -> Duration {
        let base_delay = 10u64; // seconds
        let multiplier = 3u64.pow(self.attempts);
        Duration::from_secs(base_delay * multiplier)
    }

    /// Check if job has exceeded max attempts.
    pub fn is_exhausted(&self) -> bool {
        self.attempts >= self.max_attempts
    }
}

/// Configuration for the webhook service.
#[derive(Debug, Clone)]
pub struct WebhookConfig {
    /// Redis key for the webhook job queue.
    pub queue_key: String,

    /// HTTP request timeout.
    pub request_timeout: Duration,

    /// How often to poll the queue when idle.
    pub poll_interval: Duration,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            queue_key: "ethpayserver:webhooks".to_string(),
            request_timeout: Duration::from_secs(30),
            poll_interval: Duration::from_secs(5),
        }
    }
}

impl WebhookConfig {
    /// Load configuration from environment variables.
    ///
    /// - `WEBHOOK_QUEUE_KEY` - Redis queue key (default: "ethpayserver:webhooks")
    /// - `WEBHOOK_REQUEST_TIMEOUT_SECS` - HTTP request timeout (default: 30)
    /// - `WEBHOOK_POLL_INTERVAL_SECS` - Queue poll interval (default: 5)
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
        }
    }
}

/// Webhook delivery service.
///
/// This service:
/// 1. Accepts webhook jobs via `queue_webhook()`
/// 2. Runs a background loop that delivers webhooks from the Redis queue
/// 3. Retries failed deliveries with exponential backoff
/// 4. Records delivery status in the payment_events table
pub struct WebhookService<D: WebhookDataService> {
    data_service: Arc<D>,
    redis_client: redis::Client,
    http_client: reqwest::Client,
    config: WebhookConfig,
}

impl<D: WebhookDataService + 'static> WebhookService<D> {
    /// Create a new webhook service.
    pub fn new(
        data_service: Arc<D>,
        redis_url: &str,
        config: WebhookConfig,
    ) -> Result<Self, WebhookError> {
        let redis_client =
            redis::Client::open(redis_url).map_err(|e| WebhookError::Redis(e.to_string()))?;

        let http_client = reqwest::Client::builder()
            .timeout(config.request_timeout)
            .build()
            .map_err(|e| WebhookError::Http(e.to_string()))?;

        Ok(Self {
            data_service,
            redis_client,
            http_client,
            config,
        })
    }

    /// Queue a webhook for delivery.
    ///
    /// This pushes the job to Redis for async processing.
    pub async fn queue_webhook(&self, job: WebhookJob) -> Result<(), WebhookError> {
        let mut conn = self
            .redis_client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| WebhookError::Redis(e.to_string()))?;

        let job_json =
            serde_json::to_string(&job).map_err(|e| WebhookError::Serialization(e.to_string()))?;

        redis::cmd("RPUSH")
            .arg(&self.config.queue_key)
            .arg(&job_json)
            .query_async::<i64>(&mut conn)
            .await
            .map_err(|e| WebhookError::Redis(e.to_string()))?;

        tracing::debug!(
            job_id = %job.id,
            event_type = %job.payload.event_type,
            invoice_id = %job.payload.invoice_id,
            "Queued webhook job"
        );
        metrics::record_webhook_queued(&job.payload.event_type.to_string());

        Ok(())
    }

    /// Run the webhook delivery service as a background task.
    ///
    /// This should be spawned with `tokio::spawn(service.run())`.
    pub async fn run(self: Arc<Self>) {
        tracing::info!(
            queue_key = %self.config.queue_key,
            "Starting webhook delivery service"
        );

        loop {
            match self.process_next_job().await {
                Ok(true) => {
                    // Processed a job, immediately check for more
                    continue;
                }
                Ok(false) => {
                    // No jobs in queue, wait before polling again
                    tokio::time::sleep(self.config.poll_interval).await;
                }
                Err(e) => {
                    tracing::error!(error = %e, "Error processing webhook job");
                    tokio::time::sleep(self.config.poll_interval).await;
                }
            }
        }
    }

    /// Process the next job from the queue.
    ///
    /// Returns Ok(true) if a job was processed, Ok(false) if queue was empty.
    async fn process_next_job(&self) -> Result<bool, WebhookError> {
        let mut conn = self
            .redis_client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| WebhookError::Redis(e.to_string()))?;

        // Pop job from queue (LPOP for FIFO)
        let job_json: Option<String> = redis::cmd("LPOP")
            .arg(&self.config.queue_key)
            .query_async(&mut conn)
            .await
            .map_err(|e| WebhookError::Redis(e.to_string()))?;

        let Some(json) = job_json else {
            // Queue is empty - update gauge to 0
            metrics::set_webhook_queue_depth(0);
            return Ok(false);
        };

        let mut job: WebhookJob =
            serde_json::from_str(&json).map_err(|e| WebhookError::Serialization(e.to_string()))?;

        // Check if job is scheduled for later
        if job.scheduled_at > Utc::now() {
            // Re-queue the job for later
            redis::cmd("RPUSH")
                .arg(&self.config.queue_key)
                .arg(&json)
                .query_async::<i64>(&mut conn)
                .await
                .map_err(|e| WebhookError::Redis(e.to_string()))?;
            return Ok(false);
        }

        // Attempt delivery with timing
        job.attempts += 1;
        let delivery_start = std::time::Instant::now();
        let result = self.deliver_webhook(&job).await;
        let delivery_duration = delivery_start.elapsed();

        match result {
            Ok(()) => {
                tracing::info!(
                    job_id = %job.id,
                    invoice_id = %job.payload.invoice_id,
                    attempts = job.attempts,
                    "Webhook delivered successfully"
                );
                metrics::record_webhook_delivered(&job.payload.event_type.to_string());
                metrics::record_webhook_delivery_duration(
                    &job.payload.event_type.to_string(),
                    true,
                    delivery_duration,
                );
                self.record_delivery_event(&job, true, None).await;
            }
            Err(e) => {
                tracing::warn!(
                    job_id = %job.id,
                    invoice_id = %job.payload.invoice_id,
                    attempts = job.attempts,
                    error = %e,
                    "Webhook delivery failed"
                );
                metrics::record_webhook_delivery_duration(
                    &job.payload.event_type.to_string(),
                    false,
                    delivery_duration,
                );

                if job.is_exhausted() {
                    tracing::error!(
                        job_id = %job.id,
                        invoice_id = %job.payload.invoice_id,
                        "Webhook delivery exhausted, giving up"
                    );
                    metrics::record_webhook_failed(&job.payload.event_type.to_string());
                    self.record_delivery_event(&job, false, Some(e.to_string()))
                        .await;
                } else {
                    // Schedule retry
                    job.scheduled_at =
                        Utc::now() + chrono::Duration::from_std(job.retry_delay()).unwrap();

                    let job_json = serde_json::to_string(&job)
                        .map_err(|e| WebhookError::Serialization(e.to_string()))?;

                    redis::cmd("RPUSH")
                        .arg(&self.config.queue_key)
                        .arg(&job_json)
                        .query_async::<i64>(&mut conn)
                        .await
                        .map_err(|e| WebhookError::Redis(e.to_string()))?;

                    tracing::debug!(
                        job_id = %job.id,
                        next_attempt = job.attempts + 1,
                        scheduled_at = %job.scheduled_at,
                        "Webhook scheduled for retry"
                    );
                }
            }
        }

        // Update queue depth gauge
        if let Ok(depth) = redis::cmd("LLEN")
            .arg(&self.config.queue_key)
            .query_async::<u64>(&mut conn)
            .await
        {
            metrics::set_webhook_queue_depth(depth);
        }

        Ok(true)
    }

    /// Deliver a webhook to the merchant endpoint.
    async fn deliver_webhook(&self, job: &WebhookJob) -> Result<(), WebhookError> {
        // Serialize payload
        let payload_json = serde_json::to_string(&job.payload)
            .map_err(|e| WebhookError::Serialization(e.to_string()))?;

        // Sign payload with HMAC-SHA256
        let signature = self.sign_payload(&payload_json, &job.webhook_secret);

        // Send request
        let response = self
            .http_client
            .post(&job.webhook_url)
            .header("Content-Type", "application/json")
            .header("X-Webhook-Signature", &signature)
            .header("X-Webhook-Event", job.payload.event_type.to_string())
            .header("X-Webhook-Id", job.payload.event_id.to_string())
            .body(payload_json)
            .send()
            .await
            .map_err(|e| WebhookError::Http(e.to_string()))?;

        // Check response status
        if response.status().is_success() {
            Ok(())
        } else {
            Err(WebhookError::Http(format!(
                "HTTP {} from webhook endpoint",
                response.status()
            )))
        }
    }

    /// Sign payload with HMAC-SHA256.
    fn sign_payload(&self, payload: &str, secret: &str) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        type HmacSha256 = Hmac<Sha256>;

        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
        mac.update(payload.as_bytes());
        let result = mac.finalize();

        // Return as hex string with sha256= prefix
        format!("sha256={}", hex::encode(result.into_bytes()))
    }

    /// Record webhook delivery event in payment_events table.
    async fn record_delivery_event(&self, job: &WebhookJob, success: bool, error: Option<String>) {
        let event_type = if success {
            "webhook_delivered"
        } else {
            "webhook_failed"
        };

        let event_data = serde_json::json!({
            "webhook_id": job.id,
            "event_type": job.payload.event_type.to_string(),
            "attempts": job.attempts,
            "success": success,
            "error": error,
        });

        let invoice_id = types::InvoiceId::from_string(job.payload.invoice_id.clone());

        if let Err(e) = PaymentEventWriter::create_event(
            &*self.data_service,
            &invoice_id,
            None,
            event_type,
            Some(event_data),
        )
        .await
        {
            tracing::warn!(
                error = %e,
                invoice_id = %job.payload.invoice_id,
                "Failed to record webhook delivery event"
            );
        }
    }
}

/// Error type for webhook operations.
#[derive(Debug, thiserror::Error)]
pub enum WebhookError {
    #[error("redis error: {0}")]
    Redis(String),

    #[error("http error: {0}")]
    Http(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("database error: {0}")]
    Database(#[from] types::RepositoryError),
}

/// Sign a payload with HMAC-SHA256.
///
/// Returns a signature in the format `sha256=<hex>`.
pub fn sign_webhook_payload(payload: &str, secret: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(payload.as_bytes());
    let result = mac.finalize();

    format!("sha256={}", hex::encode(result.into_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webhook_event_type_display() {
        assert_eq!(
            WebhookEventType::PaymentDetected.to_string(),
            "payment_detected"
        );
        assert_eq!(
            WebhookEventType::PaymentConfirmed.to_string(),
            "payment_confirmed"
        );
        assert_eq!(
            WebhookEventType::InvoiceExpired.to_string(),
            "invoice_expired"
        );
        assert_eq!(
            WebhookEventType::InvoiceCancelled.to_string(),
            "invoice_cancelled"
        );
        assert_eq!(WebhookEventType::LatePaid.to_string(), "late_paid");
        assert_eq!(
            WebhookEventType::RefundInitiated.to_string(),
            "refund_initiated"
        );
        assert_eq!(
            WebhookEventType::RefundConfirmed.to_string(),
            "refund_confirmed"
        );
        assert_eq!(WebhookEventType::RefundFailed.to_string(), "refund_failed");
        assert_eq!(
            WebhookEventType::PayoutInitiated.to_string(),
            "payout_initiated"
        );
        assert_eq!(
            WebhookEventType::PayoutConfirmed.to_string(),
            "payout_confirmed"
        );
        assert_eq!(WebhookEventType::PayoutFailed.to_string(), "payout_failed");
    }

    #[test]
    fn test_webhook_config_default() {
        let config = WebhookConfig::default();
        assert_eq!(config.queue_key, "ethpayserver:webhooks");
        assert_eq!(config.request_timeout, Duration::from_secs(30));
        assert_eq!(config.poll_interval, Duration::from_secs(5));
    }

    #[test]
    fn test_webhook_job_new() {
        let payload = WebhookPayload {
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
        };

        let job = WebhookJob::new(
            "https://example.com/webhook".to_string(),
            "secret123".to_string(),
            payload,
        );

        assert_eq!(job.attempts, 0);
        assert_eq!(job.max_attempts, 3);
        assert_eq!(job.webhook_url, "https://example.com/webhook");
        assert!(!job.is_exhausted());
    }

    #[test]
    fn test_webhook_job_retry_delay() {
        let payload = WebhookPayload {
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
        };

        let mut job = WebhookJob::new(
            "https://example.com/webhook".to_string(),
            "secret123".to_string(),
            payload,
        );

        // Attempt 0: 10 * 3^0 = 10 seconds
        assert_eq!(job.retry_delay(), Duration::from_secs(10));

        job.attempts = 1;
        // Attempt 1: 10 * 3^1 = 30 seconds
        assert_eq!(job.retry_delay(), Duration::from_secs(30));

        job.attempts = 2;
        // Attempt 2: 10 * 3^2 = 90 seconds
        assert_eq!(job.retry_delay(), Duration::from_secs(90));
    }

    #[test]
    fn test_webhook_job_is_exhausted() {
        let payload = WebhookPayload {
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
        };

        let mut job = WebhookJob::new(
            "https://example.com/webhook".to_string(),
            "secret123".to_string(),
            payload,
        );

        assert!(!job.is_exhausted()); // 0 attempts
        job.attempts = 1;
        assert!(!job.is_exhausted()); // 1 attempt
        job.attempts = 2;
        assert!(!job.is_exhausted()); // 2 attempts
        job.attempts = 3;
        assert!(job.is_exhausted()); // 3 attempts = max
    }

    #[test]
    fn test_sign_webhook_payload() {
        let payload = r#"{"test":"data"}"#;
        let secret = "my-secret-key";

        let signature = sign_webhook_payload(payload, secret);

        // Signature should start with "sha256="
        assert!(signature.starts_with("sha256="));

        // Signature should be deterministic
        let signature2 = sign_webhook_payload(payload, secret);
        assert_eq!(signature, signature2);

        // Different secret should produce different signature
        let signature3 = sign_webhook_payload(payload, "different-secret");
        assert_ne!(signature, signature3);

        // Different payload should produce different signature
        let signature4 = sign_webhook_payload(r#"{"test":"other"}"#, secret);
        assert_ne!(signature, signature4);
    }

    #[test]
    fn test_webhook_payload_serialization() {
        let payload = WebhookPayload {
            event_id: Uuid::new_v4(),
            event_type: WebhookEventType::PaymentConfirmed,
            timestamp: Utc::now(),
            invoice_id: "inv_123".to_string(),
            store_id: Uuid::new_v4(),
            status: "paid".to_string(),
            amount: "1000000000000000000".to_string(),
            amount_received: "1000000000000000000".to_string(),
            asset_symbol: "ETH".to_string(),
            chain_id: 1,
            network: Some("ethereum".to_string()),
            payment: Some(WebhookPaymentInfo {
                tx_hash: "0x1234".to_string(),
                from_address: Some("0xabcd".to_string()),
                block_number: Some(12345),
                confirmed: true,
            }),
        };

        // Should serialize to JSON
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("payment_confirmed"));
        assert!(json.contains("inv_123"));

        // Should deserialize back
        let deserialized: WebhookPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.invoice_id, payload.invoice_id);
        assert!(deserialized.payment.is_some());
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
