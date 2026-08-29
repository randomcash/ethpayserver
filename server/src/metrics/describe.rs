//! Metric descriptions registered with the Prometheus recorder at startup.
//!
//! Kept separate from the recording helpers in [`super::record`] so the
//! (long, purely declarative) description tables don't crowd out the logic.

use metrics::{describe_counter, describe_gauge, describe_histogram};

pub(super) fn describe_counters() {
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

pub(super) fn describe_gauges() {
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

pub(super) fn describe_histograms() {
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
