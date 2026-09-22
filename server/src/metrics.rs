//! Application metrics for ethpayserver.
//!
//! Provides application-level metrics for monitoring invoice processing,
//! payment detection, webhook delivery, and service health. Every metric goes
//! through the vendor-neutral `metrics` facade and is recorded to both
//! Prometheus (`/metrics`, scraped from outside the process) and Sentry
//! (pushed from inside, so it can raise threshold alerts) - see
//! [`FanoutRecorder`].

use metrics::{
    Counter, CounterFn, Gauge, GaugeFn, Histogram, HistogramFn, Key, KeyName, Metadata, Recorder,
    SharedString, Unit, counter, describe_counter, describe_gauge, describe_histogram, gauge,
    histogram,
};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle, PrometheusRecorder};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Global metrics handle for rendering.
static METRICS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Initialize the metrics recorder: Prometheus for `/metrics`, and Sentry's
/// Application Metrics API alongside it so gauges like
/// `payserver_chain_healthy` can raise threshold alerts.
///
/// `metrics::set_global_recorder` accepts only one recorder per process, so
/// [`FanoutRecorder`] wraps the Prometheus recorder rather than the two being
/// installed side by side - every one of the ~69 call sites across the
/// codebase keeps using the same `metrics` facade macros unchanged.
///
/// Must be called once at startup. Panics if called twice.
pub fn init_metrics() -> anyhow::Result<()> {
    let prometheus = PrometheusBuilder::new().build_recorder();
    let handle = prometheus.handle();
    metrics::set_global_recorder(FanoutRecorder { prometheus })
        .map_err(|_| anyhow::anyhow!("metrics recorder already installed"))?;

    // Store handle globally - panic if already set (programming error)
    if METRICS_HANDLE.set(handle).is_err() {
        panic!("init_metrics called twice");
    }

    // Describe all metrics
    describe_counters();
    describe_gauges();
    describe_histograms();

    Ok(())
}

/// Forwards every counter/gauge/histogram registration to the wrapped
/// Prometheus recorder - so `/metrics` renders exactly as before - and, on
/// every recorded value, also captures the same observation through Sentry's
/// metrics API. The Sentry side is vendor-specific and lives only here; every
/// call site still goes through the vendor-neutral `metrics` facade.
struct FanoutRecorder {
    prometheus: PrometheusRecorder,
}

impl Recorder for FanoutRecorder {
    fn describe_counter(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.prometheus.describe_counter(key, unit, description);
    }

    fn describe_gauge(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.prometheus.describe_gauge(key, unit, description);
    }

    fn describe_histogram(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.prometheus.describe_histogram(key, unit, description);
    }

    fn register_counter(&self, key: &Key, metadata: &Metadata<'_>) -> Counter {
        let prometheus = self.prometheus.register_counter(key, metadata);
        Counter::from_arc(Arc::new(SentryCounter {
            prometheus,
            key: key.clone(),
        }))
    }

    fn register_gauge(&self, key: &Key, metadata: &Metadata<'_>) -> Gauge {
        let prometheus = self.prometheus.register_gauge(key, metadata);
        Gauge::from_arc(Arc::new(SentryGauge {
            prometheus,
            key: key.clone(),
        }))
    }

    fn register_histogram(&self, key: &Key, metadata: &Metadata<'_>) -> Histogram {
        let prometheus = self.prometheus.register_histogram(key, metadata);
        Histogram::from_arc(Arc::new(SentryHistogram {
            prometheus,
            key: key.clone(),
        }))
    }
}

/// Sends a metric to Sentry with the key's labels attached as attributes, so
/// e.g. `payserver_chain_healthy{chain_id="1"}` can be filtered/alerted on
/// per chain in Sentry the same way it can be queried per chain in Prometheus.
fn sentry_labels(key: &Key) -> impl Iterator<Item = (String, String)> + '_ {
    key.labels()
        .map(|label| (label.key().to_string(), label.value().to_string()))
}

struct SentryCounter {
    prometheus: Counter,
    key: Key,
}

impl CounterFn for SentryCounter {
    fn increment(&self, value: u64) {
        self.prometheus.increment(value);
        let metric = sentry_labels(&self.key).fold(
            sentry::metrics::counter(self.key.name().to_string(), value as f64),
            |metric, (k, v)| metric.attribute(k, v),
        );
        metric.capture();
    }

    fn absolute(&self, value: u64) {
        // No call site sets a counter to an absolute value today, and Sentry
        // counters are delta-additive with no "set to X" verb to translate
        // this into - forward to Prometheus, which does support it, only.
        self.prometheus.absolute(value);
    }
}

struct SentryGauge {
    prometheus: Gauge,
    key: Key,
}

impl GaugeFn for SentryGauge {
    fn increment(&self, value: f64) {
        // No call site increments/decrements a gauge today (every gauge here
        // is `.set()`); Sentry gauges take a point-in-time value, and a bare
        // delta can't be turned into one without duplicating the Prometheus
        // atomic here, so only `set` is forwarded to Sentry. Warn rather than
        // silently diverge if that ever changes - a gauge feeding a threshold
        // alert (e.g. `payserver_chain_healthy`) would go stale in Sentry
        // while `/metrics` kept moving, with nothing else to say so.
        tracing::warn!(
            metric = %self.key.name(),
            "gauge.increment() is not forwarded to Sentry - only set() is"
        );
        self.prometheus.increment(value);
    }

    fn decrement(&self, value: f64) {
        tracing::warn!(
            metric = %self.key.name(),
            "gauge.decrement() is not forwarded to Sentry - only set() is"
        );
        self.prometheus.decrement(value);
    }

    fn set(&self, value: f64) {
        self.prometheus.set(value);
        let metric = sentry_labels(&self.key).fold(
            sentry::metrics::gauge(self.key.name().to_string(), value),
            |metric, (k, v)| metric.attribute(k, v),
        );
        metric.capture();
    }
}

struct SentryHistogram {
    prometheus: Histogram,
    key: Key,
}

impl HistogramFn for SentryHistogram {
    fn record(&self, value: f64) {
        self.prometheus.record(value);
        let metric = sentry_labels(&self.key).fold(
            sentry::metrics::distribution(self.key.name().to_string(), value),
            |metric, (k, v)| metric.attribute(k, v),
        );
        metric.capture();
    }
}

/// Render metrics in Prometheus format.
///
/// Returns None if metrics were not initialized.
pub fn render() -> Option<String> {
    METRICS_HANDLE.get().map(|h| h.render())
}

fn describe_counters() {
    // Invoice metrics
    describe_counter!(
        "ethpayserver_invoices_created_total",
        "Total number of invoices created"
    );
    describe_counter!(
        "ethpayserver_invoices_paid_total",
        "Total number of invoices fully paid"
    );
    describe_counter!(
        "ethpayserver_invoices_expired_total",
        "Total number of invoices that expired"
    );
    describe_counter!(
        "ethpayserver_invoices_cancelled_total",
        "Total number of invoices cancelled"
    );

    // Payment metrics
    describe_counter!(
        "ethpayserver_payments_detected_total",
        "Total number of payments detected"
    );
    describe_counter!(
        "ethpayserver_payments_confirmed_total",
        "Total number of payments confirmed"
    );

    // Webhook metrics
    describe_counter!(
        "ethpayserver_webhooks_queued_total",
        "Total number of webhooks queued"
    );
    describe_counter!(
        "ethpayserver_webhooks_delivered_total",
        "Total number of webhooks successfully delivered"
    );
    describe_counter!(
        "ethpayserver_webhooks_failed_total",
        "Total number of webhook delivery failures"
    );

    // Webhook delivery tracking
    describe_counter!(
        "ethpayserver_webhook_deliveries_total",
        "Total webhook deliveries by status (delivered, retrying, permanent_failed)"
    );

    // Store metrics
    describe_counter!(
        "ethpayserver_stores_created_total",
        "Total number of stores created"
    );

    // Rate limiting metrics
    describe_counter!(
        "ethpayserver_rate_limited_total",
        "Total number of requests rejected by rate limiting"
    );

    // Refund metrics
    //
    // No `initiated` counter here: `record_refund_initiated` was removed
    // along with its only caller when POST /invoices/{id}/refund stopped
    // creating refund rows. Registering a description for a counter nothing
    // increments would be the same "implies a capability" problem this
    // change exists to fix, just in the metrics namespace instead of the API
    // surface.
    describe_counter!(
        "ethpayserver_refunds_confirmed_total",
        "Total number of refunds confirmed"
    );
    describe_counter!(
        "ethpayserver_refunds_failed_total",
        "Total number of refunds that failed"
    );

    // Payout metrics
    describe_counter!(
        "ethpayserver_payouts_initiated_total",
        "Total number of payouts initiated"
    );
    describe_counter!(
        "ethpayserver_payouts_confirmed_total",
        "Total number of payouts confirmed"
    );
    describe_counter!(
        "ethpayserver_payouts_failed_total",
        "Total number of payouts that failed"
    );
}

fn describe_gauges() {
    describe_gauge!(
        "ethpayserver_webhook_queue_depth",
        "Current number of webhooks in the queue"
    );
    describe_gauge!(
        "ethpayserver_webhook_ready_queue_depth",
        "Number of webhooks ready for immediate delivery"
    );
    describe_gauge!(
        "ethpayserver_watched_addresses",
        "Current number of watched addresses per chain"
    );
    describe_gauge!(
        "payserver_chain_current_block",
        "Current block height on chain, as reported by the RPC source"
    );
    describe_gauge!(
        "payserver_chain_last_processed_block",
        "Last block height processed by the monitor"
    );
    describe_gauge!(
        "payserver_chain_block_lag",
        "Blocks between chain head and the last block the monitor has processed"
    );
    describe_gauge!(
        "payserver_chain_healthy",
        "Whether the chain's monitor connection is healthy (1) or not (0)"
    );
    describe_gauge!(
        "ethpayserver_registered_users",
        "Total number of registered users"
    );
    describe_gauge!("ethpayserver_stores", "Total number of stores");

    // DB pool gauges
    describe_gauge!(
        "ethpayserver_db_pool_connections",
        "Current number of connections in the DB pool by state"
    );
}

fn describe_histograms() {
    describe_histogram!(
        "ethpayserver_payment_confirmation_duration_seconds",
        "Time from payment detected to confirmed"
    );
    describe_histogram!(
        "ethpayserver_webhook_delivery_duration_seconds",
        "HTTP round-trip time per webhook delivery attempt"
    );
    describe_histogram!(
        "ethpayserver_webhook_retry_attempts",
        "Webhook delivery attempt number on failure"
    );
    describe_histogram!(
        "ethpayserver_http_request_duration_seconds",
        "API request latency"
    );
    describe_counter!("ethpayserver_http_requests_total", "Total HTTP requests");

    // DB pool histograms
    describe_histogram!(
        "ethpayserver_db_pool_wait_duration_seconds",
        "Time spent waiting to acquire a DB pool connection"
    );

    // RPC histograms and counters
    describe_histogram!(
        "ethpayserver_rpc_request_duration_seconds",
        "Duration of individual RPC calls by chain and method"
    );
    describe_counter!(
        "ethpayserver_rpc_requests_total",
        "Total RPC requests by chain, method, and status"
    );
    describe_counter!(
        "ethpayserver_rpc_errors_total",
        "Total RPC errors by chain, method, and error kind"
    );
}

// ============================================================================
// Metric recording functions
// ============================================================================

/// Record an invoice creation.
pub fn record_invoice_created(currency: &str) {
    counter!("ethpayserver_invoices_created_total", "currency" => currency.to_string())
        .increment(1);
}

/// Record an invoice paid.
pub fn record_invoice_paid() {
    counter!("ethpayserver_invoices_paid_total").increment(1);
}

/// Record an invoice expiration.
pub fn record_invoice_expired() {
    counter!("ethpayserver_invoices_expired_total").increment(1);
}

/// Record an invoice cancellation.
pub fn record_invoice_cancelled() {
    counter!("ethpayserver_invoices_cancelled_total").increment(1);
}

/// Record a payment detection.
pub fn record_payment_detected(chain_id: &types::ChainId, asset_symbol: &str) {
    counter!(
        "ethpayserver_payments_detected_total",
        "chain_id" => chain_id.to_string(),
        "asset_symbol" => asset_symbol.to_string()
    )
    .increment(1);
}

/// Record a payment confirmation.
pub fn record_payment_confirmed(chain_id: &types::ChainId, asset_symbol: &str) {
    counter!(
        "ethpayserver_payments_confirmed_total",
        "chain_id" => chain_id.to_string(),
        "asset_symbol" => asset_symbol.to_string()
    )
    .increment(1);
}

/// Record a webhook queued.
pub fn record_webhook_queued(event_type: &str) {
    counter!(
        "ethpayserver_webhooks_queued_total",
        "event_type" => event_type.to_string()
    )
    .increment(1);
}

/// Record a successful webhook delivery.
pub fn record_webhook_delivered(event_type: &str) {
    counter!(
        "ethpayserver_webhooks_delivered_total",
        "event_type" => event_type.to_string()
    )
    .increment(1);
}

/// Record a failed webhook delivery.
pub fn record_webhook_failed(event_type: &str) {
    counter!(
        "ethpayserver_webhooks_failed_total",
        "event_type" => event_type.to_string()
    )
    .increment(1);
}

/// Record a webhook delivery outcome by status (delivered, retrying, permanent_failed).
pub fn record_webhook_delivery_status(status: &str) {
    counter!(
        "ethpayserver_webhook_deliveries_total",
        "status" => status.to_string()
    )
    .increment(1);
}

/// Record a webhook retry attempt number as a histogram observation.
pub fn record_webhook_retry_attempt(attempt: u32) {
    histogram!("ethpayserver_webhook_retry_attempts").record(f64::from(attempt));
}

/// Update the webhook queue depth gauge (total jobs in ZSET).
pub fn set_webhook_queue_depth(depth: u64) {
    gauge!("ethpayserver_webhook_queue_depth").set(depth as f64);
}

/// Update the ready-queue depth gauge (jobs with `scheduled_at` <= now).
pub fn set_webhook_ready_queue_depth(depth: u64) {
    gauge!("ethpayserver_webhook_ready_queue_depth").set(depth as f64);
}

/// Update the watched addresses gauge for a chain.
pub fn set_watched_addresses(chain_id: u64, count: usize) {
    gauge!(
        "ethpayserver_watched_addresses",
        "chain_id" => chain_id.to_string()
    )
    .set(count as f64);
}

/// Update the per-chain block gauges: current head, last processed, and the
/// lag between them. `None` values (source not yet connected) are skipped
/// rather than recorded as zero, which would read as fully caught up.
pub fn set_chain_blocks(
    chain_id: u64,
    current_block: Option<u64>,
    last_processed_block: Option<u64>,
) {
    let chain_id = chain_id.to_string();

    if let Some(current) = current_block {
        gauge!("payserver_chain_current_block", "chain_id" => chain_id.clone()).set(current as f64);
    }
    if let Some(last_processed) = last_processed_block {
        gauge!("payserver_chain_last_processed_block", "chain_id" => chain_id.clone())
            .set(last_processed as f64);
    }
    if let (Some(current), Some(last_processed)) = (current_block, last_processed_block) {
        let lag = current.saturating_sub(last_processed);
        gauge!("payserver_chain_block_lag", "chain_id" => chain_id).set(lag as f64);
    }
}

/// Update the per-chain healthy gauge (1 = healthy, 0 = not).
pub fn set_chain_healthy(chain_id: u64, is_healthy: bool) {
    gauge!(
        "payserver_chain_healthy",
        "chain_id" => chain_id.to_string()
    )
    .set(if is_healthy { 1.0 } else { 0.0 });
}

/// Update the registered users gauge.
pub fn set_registered_users(count: u64) {
    gauge!("ethpayserver_registered_users").set(count as f64);
}

/// Update the stores gauge.
pub fn set_stores(count: u64) {
    gauge!("ethpayserver_stores").set(count as f64);
}

/// Record a store creation.
pub fn record_store_created() {
    counter!("ethpayserver_stores_created_total").increment(1);
}

/// Record a payout initiation.
pub fn record_payout_initiated(chain_id: &types::ChainId, asset_symbol: &str) {
    counter!(
        "ethpayserver_payouts_initiated_total",
        "chain_id" => chain_id.to_string(),
        "asset_symbol" => asset_symbol.to_string()
    )
    .increment(1);
}

/// Record a rate-limited request.
pub fn record_rate_limited(tier: &str) {
    counter!(
        "ethpayserver_rate_limited_total",
        "tier" => tier.to_string()
    )
    .increment(1);
}

// ============================================================================
// Histogram recording functions
// ============================================================================

/// Record the duration from payment detected to confirmed.
pub fn record_payment_confirmation_duration(
    chain_id: &types::ChainId,
    asset_symbol: &str,
    duration: Duration,
) {
    histogram!(
        "ethpayserver_payment_confirmation_duration_seconds",
        "chain_id" => chain_id.to_string(),
        "asset_symbol" => asset_symbol.to_string()
    )
    .record(duration.as_secs_f64());
}

/// Record a webhook delivery round-trip duration.
pub fn record_webhook_delivery_duration(event_type: &str, success: bool, duration: Duration) {
    histogram!(
        "ethpayserver_webhook_delivery_duration_seconds",
        "event_type" => event_type.to_string(),
        "status" => if success { "ok" } else { "error" }.to_string()
    )
    .record(duration.as_secs_f64());
}

/// Record an HTTP request.
pub fn record_http_request(method: &str, path: &str, status: u16, duration: Duration) {
    let labels = [
        ("method", method.to_string()),
        ("path", path.to_string()),
        ("status", status.to_string()),
    ];
    counter!("ethpayserver_http_requests_total", &labels).increment(1);
    histogram!("ethpayserver_http_request_duration_seconds", &labels)
        .record(duration.as_secs_f64());
}

// ============================================================================
// DB pool metric functions
// ============================================================================

/// Set the DB pool connections gauge for a given state (idle or used).
pub fn set_db_pool_connections(state: &str, count: u64) {
    gauge!(
        "ethpayserver_db_pool_connections",
        "state" => state.to_string()
    )
    .set(count as f64);
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrics_exporter_prometheus::PrometheusBuilder;

    /// Install a per-test Prometheus recorder and return its handle.
    ///
    /// Must be called at most once per test process. Since the global recorder
    /// is a singleton, parallel tests that each call this will race. In
    /// practice the CI runner executes server unit tests in a single thread
    /// (--test-threads=1 for lib tests) so this is safe.
    #[allow(clippy::expect_used)]
    fn test_recorder() -> metrics_exporter_prometheus::PrometheusHandle {
        let builder = PrometheusBuilder::new();
        let handle = builder.install_recorder().expect("recorder already set");
        describe_counters();
        describe_gauges();
        describe_histograms();
        handle
    }

    // Exercises `FanoutRecorder` directly, without going through
    // `metrics::set_global_recorder` (a process-wide singleton other tests
    // in this module also touch). Proves the Prometheus side - the
    // `/metrics` escape hatch this ticket promises to keep - still gets
    // every value the facade records, even though every counter/gauge/
    // histogram handle now also carries a Sentry-forwarding wrapper.
    //
    // The Sentry-forwarding side of these same handles is covered by
    // `fanout_recorder_forwards_to_sentry_with_correct_types` below.
    #[test]
    fn fanout_recorder_still_updates_prometheus() {
        let prometheus = PrometheusBuilder::new().build_recorder();
        let recorder = FanoutRecorder { prometheus };
        let metadata = Metadata::new(module_path!(), metrics::Level::INFO, None);

        let counter_key = Key::from_parts("test_fanout_counter", vec![]);
        recorder
            .register_counter(&counter_key, &metadata)
            .increment(1);

        let gauge_key = Key::from_parts(
            "test_fanout_gauge",
            vec![metrics::Label::new("chain_id", "1")],
        );
        recorder.register_gauge(&gauge_key, &metadata).set(3.0);

        let histogram_key = Key::from_parts("test_fanout_histogram", vec![]);
        recorder
            .register_histogram(&histogram_key, &metadata)
            .record(0.5);

        let output = recorder.prometheus.handle().render();
        assert!(output.contains("test_fanout_counter 1"));
        assert!(output.contains("test_fanout_gauge{chain_id=\"1\"} 3"));
        assert!(output.contains("test_fanout_histogram"));
    }

    // Proves the half of `FanoutRecorder` the test above can't: that a
    // counter, a gauge, and a histogram each reach Sentry with the right
    // `MetricType` and the label attached as an attribute -
    // `payserver_chain_healthy`/`payserver_chain_block_lag` are gauges with a
    // `chain_id` label, and an alert rule needs both the type and the
    // attribute to work.
    //
    // `sentry::test::with_captured_envelopes` binds a fresh, isolated `Hub`
    // with a `TestTransport` for the duration of the closure - it never
    // touches the process-global `Hub`, so unlike a real `sentry::init` it
    // doesn't race with `set_global_recorder`-based tests elsewhere in this
    // module or in this file's other tests.
    #[test]
    #[allow(clippy::expect_used)]
    fn fanout_recorder_forwards_to_sentry_with_correct_types() {
        let recorder = FanoutRecorder {
            prometheus: PrometheusBuilder::new().build_recorder(),
        };
        let metadata = Metadata::new(module_path!(), metrics::Level::INFO, None);

        let envelopes = sentry::test::with_captured_envelopes(|| {
            let counter_key = Key::from_parts(
                "test_sentry_counter",
                vec![metrics::Label::new("chain_id", "1")],
            );
            recorder
                .register_counter(&counter_key, &metadata)
                .increment(2);

            let gauge_key = Key::from_parts(
                "test_sentry_gauge",
                vec![metrics::Label::new("chain_id", "1")],
            );
            recorder.register_gauge(&gauge_key, &metadata).set(7.0);

            let histogram_key = Key::from_parts("test_sentry_histogram", vec![]);
            recorder
                .register_histogram(&histogram_key, &metadata)
                .record(0.25);

            // Sentry batches metrics (flushed every 100 items or 5 seconds)
            // rather than sending them immediately - without this, the
            // closure would return and the test hub would be torn down
            // before anything reached the transport.
            if let Some(client) = sentry::Hub::current().client() {
                client.flush(Some(Duration::from_secs(5)));
            }
        });

        let metrics: Vec<&sentry::protocol::Metric> = envelopes
            .iter()
            .flat_map(sentry::Envelope::items)
            .filter_map(|item| match item {
                sentry::protocol::EnvelopeItem::ItemContainer(
                    sentry::protocol::ItemContainer::Metrics(metrics),
                ) => Some(metrics.as_slice()),
                _ => None,
            })
            .flatten()
            .collect();

        let counter = metrics
            .iter()
            .find(|m| m.name.as_ref() == "test_sentry_counter")
            .expect("counter metric was not captured by Sentry");
        assert_eq!(counter.r#type, sentry::protocol::MetricType::Counter);
        assert_eq!(counter.value, 2.0);
        assert_eq!(
            counter
                .attributes
                .get("chain_id")
                .and_then(|a| a.0.as_str()),
            Some("1")
        );

        let gauge = metrics
            .iter()
            .find(|m| m.name.as_ref() == "test_sentry_gauge")
            .expect("gauge metric was not captured by Sentry");
        assert_eq!(gauge.r#type, sentry::protocol::MetricType::Gauge);
        assert_eq!(gauge.value, 7.0);
        assert_eq!(
            gauge.attributes.get("chain_id").and_then(|a| a.0.as_str()),
            Some("1")
        );

        let histogram = metrics
            .iter()
            .find(|m| m.name.as_ref() == "test_sentry_histogram")
            .expect("histogram metric was not captured by Sentry");
        assert_eq!(histogram.r#type, sentry::protocol::MetricType::Distribution);
        assert_eq!(histogram.value, 0.25);
    }

    #[test]
    fn test_db_pool_connections_gauge() {
        // This test verifies the gauge recording function doesn't panic
        // and writes the expected metric names.
        set_db_pool_connections("idle", 3);
        set_db_pool_connections("used", 7);
    }

    // Integration-style test that verifies the metrics render output.
    // Only runs when the recorder is available (e.g., not in parallel with
    // other recorder-installing tests). Gated behind an env var so it
    // doesn't conflict with other tests that install a global recorder.
    #[test]
    fn test_new_metrics_render() {
        if std::env::var("METRICS_INTEGRATION_TEST").is_err() {
            return; // skip unless explicitly enabled
        }
        let handle = test_recorder();

        // Record DB pool metrics
        set_db_pool_connections("idle", 5);
        set_db_pool_connections("used", 10);

        // Record via the histogram directly (simulating data-service)
        histogram!("ethpayserver_db_pool_wait_duration_seconds").record(0.003);
        histogram!("ethpayserver_db_pool_wait_duration_seconds").record(0.012);

        // Record RPC metrics (simulating evm crate)
        histogram!(
            "ethpayserver_rpc_request_duration_seconds",
            "chain_id" => "11155111",
            "method" => "get_block_number"
        )
        .record(0.045);

        counter!(
            "ethpayserver_rpc_requests_total",
            "chain_id" => "11155111",
            "method" => "get_block_number",
            "status" => "ok"
        )
        .increment(1);

        counter!(
            "ethpayserver_rpc_errors_total",
            "chain_id" => "11155111",
            "method" => "get_logs",
            "error_kind" => "timeout"
        )
        .increment(1);

        let output = handle.render();
        assert!(
            output.contains("ethpayserver_db_pool_connections"),
            "missing db_pool_connections"
        );
        assert!(
            output.contains("ethpayserver_db_pool_wait_duration_seconds"),
            "missing db_pool_wait_duration"
        );
        assert!(
            output.contains("ethpayserver_rpc_request_duration_seconds"),
            "missing rpc_request_duration"
        );
        assert!(
            output.contains("ethpayserver_rpc_requests_total"),
            "missing rpc_requests_total"
        );
        assert!(
            output.contains("ethpayserver_rpc_errors_total"),
            "missing rpc_errors_total"
        );

        // Reproduces the ticket's "be seen to fail" check: a chain that is
        // caught up and healthy, then one whose monitor has stalled while
        // the chain head keeps moving, must show the lag climbing and
        // health flipping to unhealthy - not a gauge stuck at its first
        // value.
        set_chain_blocks(1, Some(100), Some(100));
        set_chain_healthy(1, true);
        let output = handle.render();
        assert!(output.contains("payserver_chain_block_lag{chain_id=\"1\"} 0"));
        assert!(output.contains("payserver_chain_healthy{chain_id=\"1\"} 1"));

        // Monitor stalls: chain head advances, last_processed_block does not.
        set_chain_blocks(1, Some(103), Some(100));
        set_chain_healthy(1, false);
        let output = handle.render();
        assert!(
            output.contains("payserver_chain_block_lag{chain_id=\"1\"} 3"),
            "lag did not climb when the monitor fell behind: {output}"
        );
        assert!(
            output.contains("payserver_chain_healthy{chain_id=\"1\"} 0"),
            "healthy gauge did not flip to 0 when the chain fell behind: {output}"
        );
    }

    #[test]
    fn test_chain_gauges_do_not_panic() {
        // Mirrors test_db_pool_connections_gauge: confirms the recording
        // functions run cleanly, including the "source not connected yet"
        // case where both block numbers are `None`.
        set_chain_blocks(1, Some(100), Some(95));
        set_chain_blocks(1, None, None);
        set_chain_healthy(1, true);
        set_chain_healthy(1, false);
    }
}
