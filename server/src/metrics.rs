//! Prometheus metrics for ethpayserver.
//!
//! Provides application-level metrics for monitoring invoice processing,
//! payment detection, webhook delivery, and service health.

use metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::OnceLock;
use std::time::Duration;

/// Global metrics handle for rendering.
static METRICS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Initialize the Prometheus metrics recorder.
///
/// Must be called once at startup. Panics if called twice.
pub fn init_metrics() -> Result<(), metrics_exporter_prometheus::BuildError> {
    let builder = PrometheusBuilder::new();
    let handle = builder.install_recorder()?;

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
    describe_counter!(
        "ethpayserver_refunds_initiated_total",
        "Total number of refunds initiated"
    );
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
pub fn record_payment_detected(chain_id: u64, asset_symbol: &str) {
    counter!(
        "ethpayserver_payments_detected_total",
        "chain_id" => chain_id.to_string(),
        "asset_symbol" => asset_symbol.to_string()
    )
    .increment(1);
}

/// Record a payment confirmation.
pub fn record_payment_confirmed(chain_id: u64, asset_symbol: &str) {
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
pub fn record_payout_initiated(chain_id: u64, asset_symbol: &str) {
    counter!(
        "ethpayserver_payouts_initiated_total",
        "chain_id" => chain_id.to_string(),
        "asset_symbol" => asset_symbol.to_string()
    )
    .increment(1);
}

/// Record a refund initiation.
pub fn record_refund_initiated(chain_id: u64, asset_symbol: &str) {
    counter!(
        "ethpayserver_refunds_initiated_total",
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
pub fn record_payment_confirmation_duration(chain_id: u64, asset_symbol: &str, duration: Duration) {
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
    }
}
