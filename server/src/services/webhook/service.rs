//! Webhook delivery service: queue, delivery loop, signing, and retry handling.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use data_service::{
    PaymentEventWriter, UpsertDeliveryParams, WebhookDeliveryStatus, WebhookDeliveryWriter,
};

use crate::metrics;

use super::{WebhookConfig, WebhookError, WebhookJob};

/// Trait for data service requirements in WebhookService.
pub trait WebhookDataService: PaymentEventWriter + WebhookDeliveryWriter + Send + Sync {}

impl<T> WebhookDataService for T where T: PaymentEventWriter + WebhookDeliveryWriter + Send + Sync {}

/// The queue an emitter hands a job to.
///
/// [`WebhookService`] is the only production implementation. Emitters hold
/// this rather than the concrete service so that what they emit is
/// observable: without a seam here, the only way to see whether a handler
/// emits an event is to stand up Redis, which is why no test asserted on
/// webhook emission at all.
#[async_trait]
pub trait WebhookSink: Send + Sync {
    /// Enqueue a job for delivery.
    async fn queue(&self, job: WebhookJob) -> Result<(), WebhookError>;
}

#[async_trait]
impl<D: WebhookDataService + 'static> WebhookSink for WebhookService<D> {
    async fn queue(&self, job: WebhookJob) -> Result<(), WebhookError> {
        self.queue_webhook(job).await
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
    redis_conn: tokio::sync::OnceCell<redis::aio::ConnectionManager>,
    http_client: reqwest::Client,
    config: WebhookConfig,
}

impl<D: WebhookDataService + 'static> WebhookService<D> {
    /// Create a new webhook service.
    ///
    /// `redis::Client::open` only parses the URL; it does not connect. The
    /// actual `ConnectionManager` is established lazily, on the first call to
    /// `queue_webhook`/`process_next_job`, and cached for every call after
    /// that. A one-shot connection re-opened on every call would repeat DNS
    /// resolution and the initial handshake for every single job, so any
    /// brief hiccup in resolving the Redis hostname (a container restart, for
    /// example) failed every in-flight operation instead of just the one call
    /// that triggered the reconnect. But connecting eagerly here instead would
    /// turn that same hiccup into a startup failure for the whole server if it
    /// happens to land during boot, which is a larger blast radius than the
    /// webhook subsystem this is about — so construction stays infallible on
    /// Redis reachability, same as the plain `Client::open` this replaces.
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
            redis_conn: tokio::sync::OnceCell::new(),
            http_client,
            config,
        })
    }

    /// Get the shared connection, establishing it on first use.
    async fn connection(&self) -> Result<redis::aio::ConnectionManager, WebhookError> {
        let conn = self
            .redis_conn
            .get_or_try_init(|| async {
                redis::aio::ConnectionManager::new(self.redis_client.clone())
                    .await
                    .map_err(|e| WebhookError::Redis(e.to_string()))
            })
            .await?;
        Ok(conn.clone())
    }

    /// Queue a webhook for delivery.
    ///
    /// This adds the job to a Redis sorted set keyed by `scheduled_at` timestamp.
    pub async fn queue_webhook(&self, job: WebhookJob) -> Result<(), WebhookError> {
        let mut conn = self.connection().await?;

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

        self.write_delivery_record(&job, WebhookDeliveryStatus::Pending, 0, None)
            .await;

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
        let mut conn = self.connection().await?;

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
                self.write_delivery_record(
                    &job,
                    WebhookDeliveryStatus::Delivered,
                    job.attempts as i32,
                    None,
                )
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
                    self.write_delivery_record(
                        &job,
                        WebhookDeliveryStatus::Failed,
                        job.attempts as i32,
                        Some(error_msg.clone()),
                    )
                    .await;
                    self.record_delivery_event(&job, "webhook_permanent_failed", Some(error_msg))
                        .await;
                } else {
                    // Schedule retry.
                    metrics::record_webhook_delivery_status("retrying");
                    metrics::record_webhook_retry_attempt(job.attempts);
                    self.write_delivery_record(
                        &job,
                        WebhookDeliveryStatus::Retrying,
                        job.attempts as i32,
                        Some(error_msg.clone()),
                    )
                    .await;
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
    async fn update_queue_gauges(&self, conn: &mut redis::aio::ConnectionManager) {
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
            .header(
                "X-Webhook-Idempotency-Key",
                job.payload.idempotency_key.as_str(),
            )
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

    /// Write (insert or update) this job's row in `webhook_deliveries`.
    ///
    /// Keyed by `job.id`, so the pending insert and every later attempt of
    /// the same job update one row rather than accumulating a row per retry.
    /// Best-effort like `record_delivery_event`: a failure here must not stop
    /// or retry the delivery itself, only leave its history incomplete.
    async fn write_delivery_record(
        &self,
        job: &WebhookJob,
        status: WebhookDeliveryStatus,
        attempts: i32,
        last_error: Option<String>,
    ) {
        let payload = match serde_json::to_value(&job.payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    job_id = %job.id,
                    error = %e,
                    "Failed to serialize webhook payload for delivery record"
                );
                return;
            }
        };

        let params = UpsertDeliveryParams {
            id: job.id,
            store_webhook_id: job.store_webhook_id,
            invoice_id: job.payload.invoice_id.clone(),
            event_type: job.payload.event_type.to_string(),
            status,
            attempts,
            max_attempts: job.max_attempts as i32,
            last_error,
            payload,
        };

        if let Err(e) = WebhookDeliveryWriter::upsert_delivery(&*self.data_service, params).await {
            tracing::warn!(
                job_id = %job.id,
                error = %e,
                "Failed to record webhook delivery"
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

    fn test_payload() -> crate::services::webhook::WebhookPayload {
        use crate::services::webhook::{WebhookEventType, WebhookPayload};
        use types::{InvoiceData, InvoiceId, InvoiceStatus, StoreId};

        let invoice = InvoiceData {
            id: InvoiceId::from_string("test-invoice".to_string()),
            store_id: StoreId::new(),
            currency: "ETH".to_string(),
            status: InvoiceStatus::Expired,
            amount: "1000".to_string(),
            amount_received: "1000".to_string(),
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            metadata: None,
            customer_email: None,
            extra: None,
        };
        WebhookPayload::invoice_event(WebhookEventType::InvoiceExpired, &invoice)
    }

    /// A defect this regresses: both `queue_webhook` and `process_next_job`
    /// used to call `redis::Client::get_multiplexed_async_connection()` fresh
    /// on every invocation, so every Redis command re-resolved DNS and
    /// re-opened a TCP connection instead of reusing one. On Docker's embedded
    /// DNS resolver that shows up as an intermittent "no address associated
    /// with hostname" under nothing worse than a burst of webhook jobs — the
    /// resolver rate-limits, not the network. `ConnectionManager` (as already
    /// used elsewhere in this codebase, see `data-service/src/redis`) opens
    /// the connection once and reconnects internally, so a healthy run makes
    /// exactly one TCP connection no matter how many jobs it queues.
    ///
    /// This proxies real Redis traffic through a listener that counts
    /// accepted connections, so it needs a real Redis instance and is
    /// `#[ignore]`d like this crate's other tests that need real
    /// infrastructure. Point `TEST_REDIS_URL` at one to run it.
    ///
    /// The `test` CI job sets `TEST_REDIS_URL` alongside `DATABASE_URL`, the
    /// same way this crate's Postgres-backed `#[ignore]`d tests expect
    /// `DATABASE_URL` to be there for `--run-ignored only` runs. A developer
    /// running this locally without Redis won't have it set, though - and a
    /// missing address there used to mean spending a full `ConnectionManager`
    /// reconnect-retry cycle (minutes) discovering that the default doesn't
    /// exist either, hanging the run instead of failing it. Skip immediately
    /// when the variable isn't set; a missing dependency should be silent in
    /// seconds, not a slow, unexplained timeout.
    #[tokio::test]
    #[ignore = "requires a local Redis instance; set TEST_REDIS_URL, e.g. redis://127.0.0.1:6379"]
    async fn test_redis_connection_is_reused_across_queue_calls() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        use tokio::net::{TcpListener, TcpStream};

        let Ok(backend_addr) = std::env::var("TEST_REDIS_URL") else {
            eprintln!(
                "skipping test_redis_connection_is_reused_across_queue_calls: TEST_REDIS_URL not set"
            );
            return;
        };
        let backend_addr = backend_addr.trim_start_matches("redis://").to_string();

        // A transparent proxy in front of the real Redis instance that counts
        // how many separate TCP connections the service opens through it.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        let connection_count = Arc::new(AtomicUsize::new(0));

        {
            let connection_count = Arc::clone(&connection_count);
            tokio::spawn(async move {
                loop {
                    let Ok((mut inbound, _)) = listener.accept().await else {
                        break;
                    };
                    connection_count.fetch_add(1, Ordering::SeqCst);
                    let backend_addr = backend_addr.clone();
                    tokio::spawn(async move {
                        if let Ok(mut outbound) = TcpStream::connect(&backend_addr).await {
                            let _ =
                                tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                        }
                    });
                }
            });
        }

        let data_service = Arc::new(data_service::InMemoryDataService::new());
        let config = WebhookConfig {
            queue_key: format!("test:webhook-conn-reuse:{}", uuid::Uuid::new_v4()),
            ..WebhookConfig::default()
        };
        let service = WebhookService::new(data_service, &format!("redis://{proxy_addr}"), config)
            .expect("service should be constructed");

        for _ in 0..5 {
            let job = WebhookJob::new(
                uuid::Uuid::new_v4(),
                "https://example.com/webhook".to_string(),
                "secret".to_string(),
                test_payload(),
            );
            service
                .queue_webhook(job)
                .await
                .expect("queue_webhook should succeed");
        }

        assert_eq!(
            connection_count.load(Ordering::SeqCst),
            1,
            "expected one persistent Redis connection reused across queue_webhook calls, not one opened per call"
        );
    }

    /// A defect this regresses: an earlier version of the connection-reuse
    /// fix above made `WebhookService::new` await a live `ConnectionManager`
    /// during construction. That meant the exact DNS hiccup this service is
    /// meant to tolerate mid-run ("no address associated with hostname") took
    /// down the whole server at boot instead of just degrading webhook
    /// delivery, if it happened to land while `new` was awaiting. Construction
    /// must never touch the network — connectivity is discovered lazily, on
    /// the first real command.
    #[test]
    fn new_does_not_require_redis_to_be_reachable() {
        let data_service = Arc::new(data_service::InMemoryDataService::new());
        let config = WebhookConfig::default();

        WebhookService::new(
            data_service,
            "redis://this-host-does-not-resolve.invalid:6379",
            config,
        )
        .expect("construction must not depend on Redis being reachable");
    }
}
