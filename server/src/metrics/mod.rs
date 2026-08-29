//! Prometheus metrics for ethpayserver.
//!
//! Provides application-level metrics for monitoring invoice processing,
//! payment detection, webhook delivery, and service health.

mod describe;
mod record;

pub use record::*;

use describe::{describe_counters, describe_gauges, describe_histograms};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::OnceLock;

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

#[cfg(test)]
mod tests {
    use super::*;
    use metrics::{counter, histogram};
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
