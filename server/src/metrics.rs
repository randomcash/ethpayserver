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

    // Store metrics
    describe_counter!(
        "ethpayserver_stores_created_total",
        "Total number of stores created"
    );
}

fn describe_gauges() {
    describe_gauge!(
        "ethpayserver_webhook_queue_depth",
        "Current number of webhooks in the queue"
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
        "ethpayserver_http_request_duration_seconds",
        "API request latency"
    );
    describe_counter!("ethpayserver_http_requests_total", "Total HTTP requests");
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

/// Update the webhook queue depth gauge.
pub fn set_webhook_queue_depth(depth: u64) {
    gauge!("ethpayserver_webhook_queue_depth").set(depth as f64);
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
