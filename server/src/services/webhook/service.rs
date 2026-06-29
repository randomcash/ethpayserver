//! Webhook delivery service: queue, delivery loop, signing, and retry handling.

use std::sync::Arc;

use chrono::Utc;
use data_service::PaymentEventWriter;

use crate::metrics;

use super::{WebhookConfig, WebhookError, WebhookJob};

/// Trait for data service requirements in WebhookService.
pub trait WebhookDataService: PaymentEventWriter + Send + Sync {}

impl<T> WebhookDataService for T where T: PaymentEventWriter + Send + Sync {}

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
    /// This adds the job to a Redis sorted set keyed by `scheduled_at` timestamp.
    pub async fn queue_webhook(&self, job: WebhookJob) -> Result<(), WebhookError> {
        let mut conn = self
            .redis_client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| WebhookError::Redis(e.to_string()))?;

        let job_json =
            serde_json::to_string(&job).map_err(|e| WebhookError::Serialization(e.to_string()))?;

        let score = job.scheduled_at.timestamp() as f64;
        redis::cmd("ZADD")
            .arg(&self.config.queue_key)
            .arg(score)
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
    /// Returns Ok(true) if a job was processed, Ok(false) if queue was empty
    /// or no jobs are ready yet.
    #[allow(clippy::too_many_lines, clippy::cognitive_complexity)] // Redis dequeue + HTTP delivery + retry logic
    async fn process_next_job(&self) -> Result<bool, WebhookError> {
        let mut conn = self
            .redis_client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| WebhookError::Redis(e.to_string()))?;

        let now = Utc::now().timestamp() as f64;

        // Fetch the earliest ready job (score <= now)
        let results: Vec<String> = redis::cmd("ZRANGEBYSCORE")
            .arg(&self.config.queue_key)
            .arg("-inf")
            .arg(now)
            .arg("LIMIT")
            .arg(0)
            .arg(1)
            .query_async(&mut conn)
            .await
            .map_err(|e| WebhookError::Redis(e.to_string()))?;

        let Some(json) = results.into_iter().next() else {
            // No ready jobs — update gauges
            self.update_queue_gauges(&mut conn).await;
            return Ok(false);
        };

        // Atomically remove the job we just read
        let removed: i64 = redis::cmd("ZREM")
            .arg(&self.config.queue_key)
            .arg(&json)
            .query_async(&mut conn)
            .await
            .map_err(|e| WebhookError::Redis(e.to_string()))?;

        if removed == 0 {
            // Another worker grabbed it — try again next tick
            return Ok(false);
        }

        let mut job: WebhookJob =
            serde_json::from_str(&json).map_err(|e| WebhookError::Serialization(e.to_string()))?;

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
                metrics::record_webhook_delivery_status("delivered");
                self.record_delivery_event(&job, "webhook_delivered", None)
                    .await;
            }
            Err(e) => {
                let error_msg = truncate_error(&e.to_string(), 500);

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
                        "Webhook delivery permanently failed after {} attempts",
                        job.attempts,
                    );
                    metrics::record_webhook_failed(&job.payload.event_type.to_string());
                    metrics::record_webhook_delivery_status("permanent_failed");
                    self.record_delivery_event(&job, "webhook_permanent_failed", Some(error_msg))
                        .await;
                } else {
                    // Schedule retry.
                    metrics::record_webhook_delivery_status("retrying");
                    metrics::record_webhook_retry_attempt(job.attempts);
                    self.record_delivery_event(&job, "webhook_retrying", Some(error_msg))
                        .await;

                    // `retry_delay()` returns a bounded std Duration, so the chrono
                    // conversion cannot fail in practice.
                    #[allow(clippy::unwrap_used, reason = "retry_delay is bounded")]
                    let next = Utc::now() + chrono::Duration::from_std(job.retry_delay()).unwrap();
                    job.scheduled_at = next;

                    let job_json = serde_json::to_string(&job)
                        .map_err(|e| WebhookError::Serialization(e.to_string()))?;

                    let score = job.scheduled_at.timestamp() as f64;
                    redis::cmd("ZADD")
                        .arg(&self.config.queue_key)
                        .arg(score)
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

        // Update queue depth gauges
        self.update_queue_gauges(&mut conn).await;

        Ok(true)
    }

    /// Update both queue depth gauges (total via ZCARD, ready via ZCOUNT).
    async fn update_queue_gauges(&self, conn: &mut redis::aio::MultiplexedConnection) {
        if let Ok(depth) = redis::cmd("ZCARD")
            .arg(&self.config.queue_key)
            .query_async::<u64>(conn)
            .await
        {
            metrics::set_webhook_queue_depth(depth);
        }
        let now = Utc::now().timestamp() as f64;
        if let Ok(ready) = redis::cmd("ZCOUNT")
            .arg(&self.config.queue_key)
            .arg("-inf")
            .arg(now)
            .query_async::<u64>(conn)
            .await
        {
            metrics::set_webhook_ready_queue_depth(ready);
        }
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

        // HMAC accepts any key length — this constructor is infallible in practice.
        #[allow(
            clippy::expect_used,
            reason = "HmacSha256::new_from_slice is infallible for any key length"
        )]
        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
        mac.update(payload.as_bytes());
        let result = mac.finalize();

        // Return as hex string with sha256= prefix
        format!("sha256={}", hex::encode(result.into_bytes()))
    }

    /// Record webhook delivery event in payment_events table.
    async fn record_delivery_event(
        &self,
        job: &WebhookJob,
        event_type: &str,
        error: Option<String>,
    ) {
        let event_data = serde_json::json!({
            "webhook_id": job.id,
            "webhook_event_type": job.payload.event_type.to_string(),
            "attempts": job.attempts,
            "max_attempts": job.max_attempts,
            "last_error": error,
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

/// Truncate an error message to a maximum length, appending "..." if truncated.
fn truncate_error(error: &str, max_len: usize) -> String {
    if error.len() <= max_len {
        error.to_string()
    } else {
        // Find a safe truncation point that doesn't split a multi-byte UTF-8 char
        let mut end = max_len.saturating_sub(3);
        while end > 0 && !error.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &error[..end])
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_truncate_error() {
        let short = "short error";
        assert_eq!(truncate_error(short, 500), short);

        let exact = "a".repeat(500);
        assert_eq!(truncate_error(&exact, 500), exact);

        let long = "b".repeat(600);
        let truncated = truncate_error(&long, 500);
        assert_eq!(truncated.len(), 500);
        assert!(truncated.ends_with("..."));

        assert_eq!(truncate_error("", 500), "");
    }
}
