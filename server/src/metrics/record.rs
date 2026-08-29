//! Metric recording helpers.
//!
//! Every metric emitted by the server goes through one of these functions so
//! metric names and label sets stay in one place. The matching descriptions
//! live in [`super::describe`].

use metrics::{counter, gauge, histogram};
use std::time::Duration;

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
