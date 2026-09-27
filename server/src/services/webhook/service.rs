//! Webhook delivery service: queue, delivery loop, signing, and retry handling.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use chrono::Utc;
use data_service::{
    PaymentEventWriter, UpsertDeliveryParams, WebhookDeliveryStatus, WebhookDeliveryWriter,
};

use crate::metrics;

use super::{WebhookConfig, WebhookError, WebhookJob};

/// Whether a `process_next_job` failure is a fault worth reporting.
///
/// Only `WebhookError::Redis` is downgraded during shutdown: it's the shape
/// a connection failing because the container's network dropped out takes,
/// same as the redis-bridge subscriptions this mirrors. A `Serialization`
/// error means a job already sitting in the queue no longer deserializes —
/// a real bug, not a network teardown artifact — and must not go quiet just
/// because it happened to surface in the same window as a shutdown signal.
/// `Http` is listed here as always-a-fault for the same reason, though in
/// practice `process_next_job` never returns it: a delivery failure is
/// handled inline (logged via "Webhook delivery failed" or "...permanently
/// failed", never propagated with `?`), so this match arm exists to keep
/// the decision total rather than to gate a reachable path.
///
/// Split out from `log_process_error` so the shutdown/fault decision itself
/// is unit-testable without a live Redis connection.
fn process_error_is_fault(error: &WebhookError, shutting_down: bool) -> bool {
    match error {
        WebhookError::Redis(_) => !shutting_down,
        WebhookError::Http(_) | WebhookError::Serialization(_) | WebhookError::Database(_) => true,
    }
}

/// Atomically moves a job from the ready queue to the processing set.
///
/// Runs server-side as one command so there is no gap between "removed from
/// the ready queue" and "visible in the processing set" for a cancelled task
/// to land in — the two ZREM/ZADD calls this replaces used to leave exactly
/// that gap. Returns 1 if this call claimed the job, 0 if it was already
/// gone (another worker claimed it first, matching the old ZREM-based
/// claim's `removed == 0` case).
///
/// The processing-set member (`ARGV[2]`) is not the bare job JSON (`ARGV[1]`,
/// used to find and remove the *ready-queue* entry) — see `claim_job` for why
/// it carries a claim id.
const CLAIM_JOB_SCRIPT: &str = r"
if redis.call('ZSCORE', KEYS[1], ARGV[1]) == false then
    return 0
end
redis.call('ZREM', KEYS[1], ARGV[1])
redis.call('ZADD', KEYS[2], ARGV[3], ARGV[2])
return 1
";

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
    /// Set once the owning process has asked to stop.
    ///
    /// The job loop keeps polling regardless of errors, so a Redis failure
    /// caused by the container's network dropping out during shutdown reads
    /// identically to a real fault unless we know shutdown was asked for.
    shutting_down: AtomicBool,
}

impl<D: WebhookDataService + 'static> WebhookService<D> {
    /// Create a new webhook service.
    ///
    /// `redis::Client::open` only parses the URL; it does not connect. The
    /// actual connection is established lazily, on the first call to
    /// `queue_webhook`/`process_next_job`, and cached for every call after
    /// that — see `connection()`. Construction staying infallible on Redis
    /// reachability, same as the plain `Client::open` this replaces, matters
    /// because connecting eagerly here would turn a transient Redis hiccup
    /// into a startup failure for the whole server if it happened to land
    /// during boot, a larger blast radius than the webhook subsystem this is
    /// about.
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
            shutting_down: AtomicBool::new(false),
        })
    }

    /// Get the shared connection, establishing it on first use.
    ///
    /// A one-shot connection re-opened on every call would repeat DNS
    /// resolution and the initial handshake for every single job, so any
    /// brief hiccup in resolving the Redis hostname (a container restart, for
    /// example) failed every in-flight operation instead of just the one call
    /// that triggered the reconnect. `ConnectionManager` opens the connection
    /// once and reconnects internally instead.
    ///
    /// The connection attempt is bounded by `config.connect_timeout` and
    /// retried once (`set_number_of_retries(1)`): `ConnectionManager`'s own
    /// default has no connection timeout at all, so a Redis that accepts the
    /// TCP handshake but never completes the protocol handshake can leave a
    /// caller waiting indefinitely instead of failing. `get_or_try_init`
    /// below doesn't cache that failure, so a bounded timeout here is what
    /// actually turns a bad first attempt into "try again next call" instead
    /// of "hang this call forever".
    async fn connection(&self) -> Result<redis::aio::ConnectionManager, WebhookError> {
        let conn = self
            .redis_conn
            .get_or_try_init(|| async {
                let config = redis::aio::ConnectionManagerConfig::new()
                    .set_connection_timeout(self.config.connect_timeout)
                    .set_number_of_retries(1);
                redis::aio::ConnectionManager::new_with_config(self.redis_client.clone(), config)
                    .await
                    .map_err(|e| WebhookError::Redis(e.to_string()))
            })
            .await?;
        Ok(conn.clone())
    }

    /// Redis key for the in-flight processing set.
    ///
    /// A job lives here, scored by its visibility deadline, from the moment
    /// it's claimed until delivery is either recorded as delivered or
    /// rescheduled — see [`CLAIM_JOB_SCRIPT`] and `reclaim_expired_jobs`.
    ///
    /// Members here are *not* the bare job JSON: each is `"{claim_id}:{json}"`
    /// (see `claim_job`), so two claims of the same content — the original
    /// claimant and whoever reclaims and re-claims it after a visibility
    /// timeout — never collide on the same member.
    fn processing_key(&self) -> String {
        format!("{}:processing", self.config.queue_key)
    }

    /// Claim a ready job by atomically moving it into the processing set.
    ///
    /// Returns the processing-set member on success, `None` if another
    /// worker claimed it first (`CLAIM_JOB_SCRIPT` returned 0).
    ///
    /// The member is `"{claim_id}:{json}"`, not the bare job JSON. Keying by
    /// content alone let a stalled claim's *own* eventual `clear_processing`
    /// call delete a different worker's active claim: worker A claims job J,
    /// stalls past `visibility_timeout`, gets reclaimed (J goes back to the
    /// ready queue under the same JSON), worker C claims that same JSON
    /// fresh, and then A finally finishes and clears "J" — which, keyed by
    /// content, is now C's live entry, not A's already-reclaimed one.
    /// Deleting C's entry that way leaves nothing to redeliver from if C is
    /// then killed mid-delivery — the exact loss this claim/reclaim scheme
    /// exists to prevent, just shifted one cycle later. A fresh claim id per
    /// claim makes A's and C's members distinct strings, so A's belated
    /// `ZREM` of its own member is a no-op once that member has already been
    /// reclaimed, and never touches C's.
    async fn claim_job(
        &self,
        conn: &mut redis::aio::ConnectionManager,
        json: &str,
        deadline: f64,
    ) -> Result<Option<String>, WebhookError> {
        let claim_id = uuid::Uuid::new_v4();
        let processing_member = format!("{claim_id}:{json}");
        let claimed: i64 = redis::Script::new(CLAIM_JOB_SCRIPT)
            .key(&self.config.queue_key)
            .key(self.processing_key())
            .arg(json)
            .arg(&processing_member)
            .arg(deadline)
            .invoke_async(conn)
            .await
            .map_err(|e| WebhookError::Redis(e.to_string()))?;
        Ok((claimed == 1).then_some(processing_member))
    }

    /// Return jobs whose visibility deadline has passed to the ready queue.
    ///
    /// A job only leaves the processing set once its delivery outcome is
    /// recorded; one still there past its deadline was claimed by a worker
    /// that never got that far — killed mid-delivery, most commonly by the
    /// `abort()` shutdown takes after its grace period. Re-adding it here
    /// with a ready score is what makes it deliverable again instead of
    /// silently gone. The corresponding processing entry is left for the
    /// claim script to remove when the job is next claimed; if this call is
    /// itself interrupted, the job is unaffected — it just remains in the
    /// processing set to be reclaimed again next tick.
    async fn reclaim_expired_jobs(&self, conn: &mut redis::aio::ConnectionManager) {
        let now = Utc::now().timestamp() as f64;
        let expired: Vec<String> = match redis::cmd("ZRANGEBYSCORE")
            .arg(self.processing_key())
            .arg("-inf")
            .arg(now)
            .query_async(conn)
            .await
        {
            Ok(jobs) => jobs,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to check for abandoned webhook jobs");
                return;
            }
        };

        for processing_member in expired {
            self.reclaim_one_expired_job(conn, &processing_member, now)
                .await;
        }
    }

    /// Return a single abandoned job to the ready queue.
    ///
    /// Split out of `reclaim_expired_jobs` purely to keep that loop's
    /// cognitive complexity down; the two-step requeue-then-clear it does is
    /// unchanged.
    ///
    /// `processing_member` is `"{claim_id}:{json}"` (see `claim_job`); the
    /// claim id is stripped before the job's plain JSON goes back onto the
    /// ready queue, since that queue matches by content alone. A member with
    /// no `:` would mean something else wrote to this set — there is no other
    /// writer — so it's dropped with a warning rather than requeued
    /// malformed.
    async fn reclaim_one_expired_job(
        &self,
        conn: &mut redis::aio::ConnectionManager,
        processing_member: &str,
        now: f64,
    ) {
        let Some((_claim_id, json)) = processing_member.split_once(':') else {
            tracing::warn!("Abandoned webhook processing entry has no claim id, dropping it");
            return;
        };
        tracing::warn!("Reclaiming abandoned webhook job for redelivery");
        let requeued = redis::cmd("ZADD")
            .arg(&self.config.queue_key)
            .arg(now)
            .arg(json)
            .query_async::<i64>(conn)
            .await;
        if let Err(e) = requeued {
            tracing::warn!(error = %e, "Failed to requeue abandoned webhook job");
            return;
        }
        self.clear_processing(conn, processing_member).await;
    }

    /// Remove a claim from the processing set once its delivery outcome —
    /// delivered, permanently failed, or rescheduled — has been recorded.
    ///
    /// `processing_member` must be the exact `"{claim_id}:{json}"` string
    /// this claim was given by `claim_job` (or reclaimed with), not the bare
    /// job JSON — see `claim_job` for why the id matters: a `ZREM` by content
    /// alone can hit a different, later claim of the same job.
    ///
    /// Best-effort like `write_delivery_record`: the delivery outcome is
    /// already decided and (for a retry) already back on the ready queue, so
    /// a failure here only risks a duplicate reclaim-triggered redelivery
    /// once the visibility deadline passes, not data loss — the same
    /// at-least-once shape this service already documents for retries.
    async fn clear_processing(
        &self,
        conn: &mut redis::aio::ConnectionManager,
        processing_member: &str,
    ) {
        if let Err(e) = redis::cmd("ZREM")
            .arg(self.processing_key())
            .arg(processing_member)
            .query_async::<i64>(conn)
            .await
        {
            tracing::warn!(error = %e, "Failed to clear delivered webhook job from processing set");
        }
    }

    /// Mark this service as shutting down intentionally.
    ///
    /// Call this from the process's own shutdown handler. A job-loop error
    /// logged afterwards is downgraded from `error` to `info` — it is the
    /// expected shape of a container being torn down, not a fault.
    pub fn begin_shutdown(&self) {
        self.shutting_down.store(true, Ordering::Relaxed);
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
                    self.log_process_error(&e);
                    tokio::time::sleep(self.config.poll_interval).await;
                }
            }
        }
    }

    /// Log a `process_next_job` failure at the level its cause deserves.
    ///
    /// During an intentional shutdown, the container's network can drop out
    /// from under this loop's Redis connection, which fails identically to a
    /// real fault. Only the unrequested case should reach Sentry.
    fn log_process_error(&self, e: &WebhookError) {
        if process_error_is_fault(e, self.shutting_down.load(Ordering::Relaxed)) {
            tracing::error!(error = %e, "Error processing webhook job");
        } else {
            tracing::info!(error = %e, "Webhook job processing failed during shutdown");
        }
    }

    /// Process the next job from the queue.
    ///
    /// Returns Ok(true) if a job was processed, Ok(false) if queue was empty
    /// or no jobs are ready yet.
    #[allow(clippy::too_many_lines, clippy::cognitive_complexity)] // Redis dequeue + HTTP delivery + retry logic
    async fn process_next_job(&self) -> Result<bool, WebhookError> {
        let mut conn = self.connection().await?;

        self.reclaim_expired_jobs(&mut conn).await;

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

        // Atomically move the job we just read into the processing set,
        // rather than removing it outright: this is the claim, and until
        // delivery is recorded one way or another the job stays visible
        // there, redeliverable if this worker is killed mid-delivery.
        // `claim_job` gives this claim its own identity so a later reclaim
        // of the same content, and a fresh claim of it by another worker,
        // can never be confused with this one — see its doc comment. `None`
        // here is "another worker grabbed it first"; safe to read as that
        // rather than "our own claim got replayed", since `ConnectionManager`
        // reconnects in the background on a dropped connection but returns
        // that error to the caller rather than silently retrying the
        // in-flight command, so this cannot execute twice for one call.
        let deadline = now + self.config.visibility_timeout.as_secs_f64();
        let Some(processing_member) = self.claim_job(&mut conn, &json, deadline).await? else {
            // Another worker grabbed it — try again next tick
            return Ok(false);
        };

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
                self.clear_processing(&mut conn, &processing_member).await;
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
                    self.clear_processing(&mut conn, &processing_member).await;
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

                    // The retry is on the ready queue under its own (updated)
                    // JSON now, so the original claim can be cleared from the
                    // processing set — see `clear_processing`.
                    self.clear_processing(&mut conn, &processing_member).await;

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
    use std::time::Duration;

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

    #[test]
    fn redis_error_during_shutdown_is_not_a_fault() {
        assert!(!process_error_is_fault(
            &WebhookError::Redis("boom".to_string()),
            true
        ));
    }

    #[test]
    fn redis_error_without_shutdown_is_a_fault() {
        assert!(process_error_is_fault(
            &WebhookError::Redis("boom".to_string()),
            false
        ));
    }

    #[test]
    fn serialization_error_during_shutdown_is_still_a_fault() {
        // A malformed job already in the queue isn't a network-teardown
        // artifact, so it must not go quiet just because a shutdown signal
        // happened to arrive in the same window.
        assert!(process_error_is_fault(
            &WebhookError::Serialization("bad json".to_string()),
            true
        ));
    }

    /// Runs `log_process_error` under a subscriber that captures its output,
    /// so the test below exercises the real `Ordering::Relaxed` load and
    /// `tracing::error!`/`tracing::info!` call sites — not just the extracted
    /// `process_error_is_fault` boolean.
    fn capture_log_process_error(shutting_down: bool) -> String {
        use std::io;
        use std::sync::Mutex;

        #[derive(Clone, Default)]
        struct Buf(Arc<Mutex<Vec<u8>>>);

        impl io::Write for Buf {
            fn write(&mut self, data: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buf {
            type Writer = Buf;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let buf = Buf::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_ansi(false)
            .finish();

        let service = WebhookService::new(
            Arc::new(data_service::InMemoryDataService::default()),
            "redis://127.0.0.1:1",
            WebhookConfig::default(),
        )
        .expect("valid redis URL");
        if shutting_down {
            service.begin_shutdown();
        }

        tracing::subscriber::with_default(subscriber, || {
            service.log_process_error(&WebhookError::Redis("boom".to_string()));
        });

        String::from_utf8(buf.0.lock().unwrap().clone()).expect("utf8 log output")
    }

    #[test]
    fn log_process_error_reports_error_when_not_shutting_down() {
        let output = capture_log_process_error(false);
        assert!(output.contains("ERROR"), "expected ERROR, got: {output}");
    }

    #[test]
    fn log_process_error_reports_info_when_shutting_down() {
        let output = capture_log_process_error(true);
        assert!(output.contains("INFO"), "expected INFO, got: {output}");
        assert!(
            !output.contains("ERROR"),
            "shutdown noise must not reach error level: {output}"
        );
    }
    /// A TCP listener that accepts connections but never writes a byte back,
    /// the same shape as a Redis that is up but wedged (or a firewall
    /// dropping packets silently): the client gets a connection, then
    /// nothing. This is what `connect_timeout` exists to bound.
    async fn spawn_unresponsive_server() -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback listener");
        let addr = listener.local_addr().expect("local_addr");
        tokio::spawn(async move {
            // Held for the task's lifetime so the sockets stay open without
            // ever being read from or written to.
            let mut held = Vec::new();
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    return;
                };
                held.push(socket);
            }
        });
        addr
    }

    /// A defect this regresses: `ConnectionManager::new`'s default config has
    /// no connection timeout at all, so a Redis that accepts the TCP
    /// handshake but never completes the protocol handshake stalls the
    /// caller indefinitely instead of failing — in this environment, a closed
    /// port took over 470s to time out with no bound configured. Bounding
    /// `connection()` from `config.connect_timeout` is what turns that into a
    /// fast, named failure, and doing it from the *config* rather than a
    /// hardcoded constant is what keeps `WEBHOOK_REDIS_CONNECT_TIMEOUT_SECS`
    /// a real knob instead of an orphaned one.
    #[tokio::test]
    async fn connection_is_bounded_by_the_configured_timeout() {
        let addr = spawn_unresponsive_server().await;
        let config = WebhookConfig {
            connect_timeout: Duration::from_millis(200),
            ..WebhookConfig::default()
        };
        let service = WebhookService::new(
            Arc::new(data_service::InMemoryDataService::new()),
            &format!("redis://{addr}"),
            config,
        )
        .expect("construct service against unresponsive listener");

        let start = std::time::Instant::now();
        assert!(
            service.connection().await.is_err(),
            "connection to an unresponsive Redis must fail, not hang"
        );
        let elapsed = start.elapsed();

        // Bounded by roughly two attempts (`set_number_of_retries(1)`) at
        // 200ms each, plus a small backoff between them — nowhere near the
        // multi-minute hang this regresses.
        assert!(
            elapsed < Duration::from_secs(2),
            "connect_timeout did not bound the connection attempt: took {elapsed:?}"
        );
    }

    /// Needs a real Redis and is `#[ignore]`d, matching the convention used
    /// for tests that need a real Postgres elsewhere in this crate. Reads
    /// `TEST_REDIS_URL`, the same variable CI's `test` job sets to point at
    /// the `redis` service it provisions alongside Postgres.
    #[tokio::test]
    #[ignore = "requires a local Redis instance; set TEST_REDIS_URL, e.g. redis://127.0.0.1:6379"]
    async fn connection_succeeds_and_is_reusable_against_a_real_redis() {
        let redis_url = std::env::var("TEST_REDIS_URL")
            .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        let service = WebhookService::new(
            Arc::new(data_service::InMemoryDataService::new()),
            &redis_url,
            WebhookConfig::default(),
        )
        .expect("construct service against TEST_REDIS_URL");

        service
            .connection()
            .await
            .expect("first connection to a reachable Redis must succeed");
        service
            .connection()
            .await
            .expect("shared connection must be reusable for a second call");
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

    /// A defect this regresses: `queue_webhook` and `process_next_job` used to
    /// open a fresh connection on every call, so every Redis command
    /// re-resolved DNS and re-opened a TCP connection instead of reusing one.
    /// On Docker's embedded DNS resolver that shows up as an intermittent "no
    /// address associated with hostname" under nothing worse than a burst of
    /// webhook jobs — the resolver rate-limits, not the network.
    /// `ConnectionManager` opens the connection once and reconnects
    /// internally, so a healthy run makes exactly one TCP connection no
    /// matter how many jobs it queues.
    ///
    /// This proxies real Redis traffic through a listener that counts
    /// accepted connections, so it needs a real Redis instance and is
    /// `#[ignore]`d like this crate's other tests that need real
    /// infrastructure. Point `TEST_REDIS_URL` at one to run it.
    #[tokio::test]
    #[ignore = "requires a local Redis instance; set TEST_REDIS_URL, e.g. redis://127.0.0.1:6379"]
    async fn test_redis_connection_is_reused_across_queue_calls() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        use tokio::net::{TcpListener, TcpStream};

        // A missing var must fail this test, not quietly no-op it: an
        // `#[ignore]`d test that returns early on a missing env var reports as
        // a pass, so a CI wiring regression that drops `TEST_REDIS_URL` would
        // go green while proving nothing about connection reuse.
        let backend_addr = std::env::var("TEST_REDIS_URL")
            .expect("set TEST_REDIS_URL to run this test, e.g. redis://127.0.0.1:6379");
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

    /// A defect this regresses: `get_or_try_init` only caches a *successful*
    /// connection attempt. If `connection()` instead cached the failure too
    /// (e.g. by using `get_or_init` with a panicking initializer, or storing
    /// the error alongside the cell), a transient failure on the very first
    /// call would wedge webhook delivery for the rest of the process's life
    /// instead of healing itself on the next call. The reuse test above never
    /// exercises this because its backend is healthy from the start.
    ///
    /// This proxies to a real Redis but drops the first connection attempt
    /// outright (accepts the TCP connection, then closes it), so the first
    /// `queue_webhook` call must fail. It only starts forwarding to the real
    /// backend after that, so the second call proves recovery rather than
    /// coincidence. `connect_timeout` is what keeps the failing attempt from
    /// stalling the test for minutes instead of failing outright — a
    /// connection that never completes its handshake has no other bound on
    /// how long a caller waits for it.
    #[tokio::test]
    #[ignore = "requires a local Redis instance; set TEST_REDIS_URL, e.g. redis://127.0.0.1:6379"]
    async fn test_connection_recovers_after_a_failed_first_attempt() {
        use std::sync::atomic::{AtomicBool, Ordering};

        use tokio::net::{TcpListener, TcpStream};

        // See the sibling reuse test above: an early return on a missing env
        // var reports as a pass for an `#[ignore]`d test, so this must fail
        // loudly instead.
        let backend_addr = std::env::var("TEST_REDIS_URL")
            .expect("set TEST_REDIS_URL to run this test, e.g. redis://127.0.0.1:6379");
        let backend_addr = backend_addr.trim_start_matches("redis://").to_string();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        let forwarding = Arc::new(AtomicBool::new(false));

        {
            let forwarding = Arc::clone(&forwarding);
            tokio::spawn(async move {
                loop {
                    let Ok((inbound, _)) = listener.accept().await else {
                        break;
                    };
                    if !forwarding.load(Ordering::SeqCst) {
                        // Simulate a connection attempt that never completes:
                        // accept, then hang up immediately.
                        drop(inbound);
                        continue;
                    }
                    let backend_addr = backend_addr.clone();
                    tokio::spawn(async move {
                        let mut inbound = inbound;
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
            queue_key: format!("test:webhook-conn-recovery:{}", uuid::Uuid::new_v4()),
            connect_timeout: Duration::from_secs(1),
            ..WebhookConfig::default()
        };
        let service = WebhookService::new(data_service, &format!("redis://{proxy_addr}"), config)
            .expect("service should be constructed");

        let new_job = || {
            WebhookJob::new(
                uuid::Uuid::new_v4(),
                "https://example.com/webhook".to_string(),
                "secret".to_string(),
                test_payload(),
            )
        };

        let first = service.queue_webhook(new_job()).await;
        assert!(
            first.is_err(),
            "the first call, against a backend that drops the connection, must fail"
        );

        forwarding.store(true, Ordering::SeqCst);

        let second = service.queue_webhook(new_job()).await;
        assert!(
            second.is_ok(),
            "a later call must recover once the backend is reachable, not replay the earlier failure forever: {:?}",
            second.err()
        );
    }

    /// A defect this regresses: an earlier version of the connection-reuse
    /// fix made `WebhookService::new` await a live `ConnectionManager` during
    /// construction. That meant the exact DNS hiccup this service is meant to
    /// tolerate mid-run ("no address associated with hostname") took down the
    /// whole server at boot instead of just degrading webhook delivery, if it
    /// happened to land while `new` was awaiting. Construction must never
    /// touch the network — connectivity is discovered lazily, on the first
    /// real command.
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

    /// A defect this regresses: `process_next_job` used to `ZREM` a job from
    /// the ready queue before attempting delivery, so a task cancellation
    /// mid-flight — the same shape `abort()` produces when the shutdown
    /// grace period elapses — dropped the job with nothing anywhere left to
    /// redeliver it from: gone from Redis, no delivery, no retry.
    ///
    /// This proves the claim-then-acknowledge replacement: the job is moved
    /// to a processing set rather than deleted, so cancelling the worker
    /// while a claim is mid-delivery leaves it there, and it comes back to
    /// the ready queue once its visibility deadline passes instead of
    /// vanishing.
    ///
    /// Needs a real Redis and is `#[ignore]`d like this crate's other tests
    /// that need real infrastructure.
    #[tokio::test]
    #[ignore = "requires a local Redis instance; set TEST_REDIS_URL, e.g. redis://127.0.0.1:6379"]
    #[allow(clippy::too_many_lines)] // claim, poll-for-claim, abort, reclaim, and three separate Redis assertions
    async fn a_job_cancelled_mid_delivery_is_reclaimed_not_lost() {
        let redis_url = std::env::var("TEST_REDIS_URL")
            .expect("set TEST_REDIS_URL to run this test, e.g. redis://127.0.0.1:6379");

        // Accepts the delivery attempt and never responds, so the worker
        // task is guaranteed to still be inside `deliver_webhook`'s HTTP
        // call — neither finished nor failed — at the moment it's aborted
        // below.
        let addr = spawn_unresponsive_server().await;

        let data_service = Arc::new(data_service::InMemoryDataService::new());
        // Zero rather than a short-but-nonzero duration: scores are seconds
        // truncated from `Utc::now().timestamp()`, so a sub-second timeout
        // would round-trip through that truncation unpredictably and make
        // "has the deadline passed" a race against the wall-clock second
        // boundary instead of a deterministic check.
        let config = WebhookConfig {
            queue_key: format!("test:webhook-cancel-mid-delivery:{}", uuid::Uuid::new_v4()),
            request_timeout: Duration::from_secs(30),
            visibility_timeout: Duration::ZERO,
            ..WebhookConfig::default()
        };
        let service = Arc::new(
            WebhookService::new(Arc::clone(&data_service), &redis_url, config)
                .expect("service should be constructed"),
        );

        let job = WebhookJob::new(
            uuid::Uuid::new_v4(),
            format!("http://{addr}/webhook"),
            "secret".to_string(),
            test_payload(),
        );
        let job_json = serde_json::to_string(&job).expect("job serializes");
        service
            .queue_webhook(job)
            .await
            .expect("queue_webhook should succeed");

        // Drive one iteration of the loop in its own task, so it can be
        // aborted exactly like the real worker task is at shutdown.
        let worker = {
            let service = Arc::clone(&service);
            tokio::spawn(async move { service.process_next_job().await })
        };
        // Poll for the claim to land instead of guessing a fixed delay: a
        // fixed sleep short enough to stay well under the endpoint's silence
        // could still elapse before the claim's single Redis round trip
        // finishes under CI load, aborting the worker before it ever
        // reaches the HTTP call this test means to interrupt — failing for
        // the wrong reason (job never left the ready queue) rather than the
        // one it's meant to test.
        let mut conn = service.connection().await.expect("connection");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let still_ready: Option<f64> = redis::cmd("ZSCORE")
                    .arg(&service.config.queue_key)
                    .arg(&job_json)
                    .query_async(&mut conn)
                    .await
                    .expect("ZSCORE queue");
                if still_ready.is_none() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("claim did not complete within 5s");

        worker.abort();
        let _ = worker.await;

        let still_queued: Option<f64> = redis::cmd("ZSCORE")
            .arg(&service.config.queue_key)
            .arg(&job_json)
            .query_async(&mut conn)
            .await
            .expect("ZSCORE queue");
        assert!(
            still_queued.is_none(),
            "the job should have left the ready queue once claimed"
        );

        let in_processing = processing_member_for(&mut conn, &service.processing_key(), &job_json)
            .await
            .is_some();
        assert!(
            in_processing,
            "a cancelled claim must leave the job in the processing set, not delete it outright"
        );

        // The visibility deadline (`visibility_timeout` of zero) has already
        // passed the instant it was set, so reclaiming needs no further wait.
        service.reclaim_expired_jobs(&mut conn).await;

        let requeued: Option<f64> = redis::cmd("ZSCORE")
            .arg(&service.config.queue_key)
            .arg(&job_json)
            .query_async(&mut conn)
            .await
            .expect("ZSCORE queue after reclaim");
        assert!(
            requeued.is_some(),
            "an abandoned job must be returned to the ready queue, deliverable again"
        );

        let still_in_processing =
            processing_member_for(&mut conn, &service.processing_key(), &job_json)
                .await
                .is_some();
        assert!(
            !still_in_processing,
            "the reclaimed job must be cleared from the processing set"
        );
    }

    /// Finds the processing-set member for a job's plain JSON, i.e. the
    /// entry whose content after the `"{claim_id}:"` prefix (see
    /// `WebhookService::claim_job`) matches exactly. Members in that set are
    /// never the bare JSON itself, so a plain `ZSCORE` lookup by content
    /// (as used against the *ready* queue elsewhere in these tests) cannot
    /// find them.
    async fn processing_member_for(
        conn: &mut redis::aio::ConnectionManager,
        key: &str,
        json: &str,
    ) -> Option<String> {
        let members: Vec<String> = redis::cmd("ZRANGE")
            .arg(key)
            .arg(0)
            .arg(-1)
            .query_async(conn)
            .await
            .expect("ZRANGE processing set");
        members
            .into_iter()
            .find(|member| member.split_once(':').map(|(_, j)| j) == Some(json))
    }

    /// A defect this regresses: before a claim carried its own id, the
    /// processing set keyed entries by job content alone. A claim that
    /// stalls past its visibility timeout gets reclaimed, and the same
    /// content is then claimed fresh by a different worker; if the stalled
    /// claim finally finishes and clears what it still believes is its own
    /// entry — the unmodified job JSON — that `ZREM` matched the *new*
    /// claim's entry too, since both were keyed by identical content.
    /// Deleting a live claim that way reintroduces total job loss if that
    /// claim is then itself cancelled: the exact defect this claim/reclaim
    /// scheme exists to fix, just shifted one reclaim cycle later. A fresh
    /// claim id per claim (see `WebhookService::claim_job`) makes the two
    /// claims distinct strings, so the stale clear can only ever remove its
    /// own, already-reclaimed entry.
    ///
    /// Needs a real Redis and is `#[ignore]`d like this crate's other tests
    /// that need real infrastructure.
    #[tokio::test]
    #[ignore = "requires a local Redis instance; set TEST_REDIS_URL, e.g. redis://127.0.0.1:6379"]
    async fn a_stale_claim_completing_after_reclaim_does_not_delete_a_newer_claim() {
        let redis_url = std::env::var("TEST_REDIS_URL")
            .expect("set TEST_REDIS_URL to run this test, e.g. redis://127.0.0.1:6379");

        let data_service = Arc::new(data_service::InMemoryDataService::new());
        let config = WebhookConfig {
            queue_key: format!("test:webhook-stale-claim:{}", uuid::Uuid::new_v4()),
            ..WebhookConfig::default()
        };
        let service = WebhookService::new(Arc::clone(&data_service), &redis_url, config)
            .expect("service should be constructed");

        let job = WebhookJob::new(
            uuid::Uuid::new_v4(),
            "https://example.com/webhook".to_string(),
            "secret".to_string(),
            test_payload(),
        );
        let job_json = serde_json::to_string(&job).expect("job serializes");
        service
            .queue_webhook(job)
            .await
            .expect("queue_webhook should succeed");

        let mut conn = service.connection().await.expect("connection");

        // Worker A claims the job with a deadline already in the past, so
        // it is immediately eligible for reclaim with no wait needed.
        let claim_a = service
            .claim_job(&mut conn, &job_json, 0.0)
            .await
            .expect("claim should succeed")
            .expect("job should be claimable");

        // A's delivery stalls past the deadline; a reclaim — run by any
        // worker, possibly A itself on a later poll — returns the job to
        // the ready queue and clears A's now-abandoned entry.
        service.reclaim_expired_jobs(&mut conn).await;

        // Worker C claims the same content fresh, with a deadline far
        // enough out that this claim is still live.
        let far_future = Utc::now().timestamp() as f64 + 300.0;
        let claim_c = service
            .claim_job(&mut conn, &job_json, far_future)
            .await
            .expect("claim should succeed")
            .expect("job should be claimable again after reclaim");
        assert_ne!(
            claim_a, claim_c,
            "two claims of the same content must get distinct processing-set identities"
        );

        // A, unaware it was reclaimed, finally finishes its stalled
        // delivery and clears what it still believes is its own claim.
        service.clear_processing(&mut conn, &claim_a).await;

        // C's claim — the one actually in flight — must be untouched.
        let c_still_claimed: Option<f64> = redis::cmd("ZSCORE")
            .arg(service.processing_key())
            .arg(&claim_c)
            .query_async(&mut conn)
            .await
            .expect("ZSCORE processing");
        assert!(
            c_still_claimed.is_some(),
            "a stale claim's belated clear must not delete a different, live claim of the same job"
        );
    }
}
