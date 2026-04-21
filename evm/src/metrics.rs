//! RPC metrics recording helpers.
//!
//! Uses the `metrics` crate's global recorder (installed by the server at startup)
//! to emit per-RPC-call counters and histograms.

use crate::error::EvmError;
use metrics::{counter, histogram};
use std::time::{Duration, Instant};

/// Record a successful RPC call.
pub fn record_rpc_ok(chain_id: u64, method: &str, duration: Duration) {
    let chain = chain_id.to_string();
    histogram!(
        "ethpayserver_rpc_request_duration_seconds",
        "chain_id" => chain.clone(),
        "method" => method.to_string()
    )
    .record(duration.as_secs_f64());

    counter!(
        "ethpayserver_rpc_requests_total",
        "chain_id" => chain,
        "method" => method.to_string(),
        "status" => "ok"
    )
    .increment(1);
}

/// Record a failed RPC call with error classification.
pub fn record_rpc_err(chain_id: u64, method: &str, duration: Duration, error: &EvmError) {
    let chain = chain_id.to_string();
    histogram!(
        "ethpayserver_rpc_request_duration_seconds",
        "chain_id" => chain.clone(),
        "method" => method.to_string()
    )
    .record(duration.as_secs_f64());

    counter!(
        "ethpayserver_rpc_requests_total",
        "chain_id" => chain.clone(),
        "method" => method.to_string(),
        "status" => "err"
    )
    .increment(1);

    let error_kind = classify_error(error);
    counter!(
        "ethpayserver_rpc_errors_total",
        "chain_id" => chain,
        "method" => method.to_string(),
        "error_kind" => error_kind
    )
    .increment(1);
}

/// Classify an EvmError into a metrics-friendly error kind.
fn classify_error(error: &EvmError) -> &'static str {
    let msg = error.to_string().to_lowercase();
    if msg.contains("timeout") || msg.contains("timed out") {
        "timeout"
    } else if msg.contains("rate limit") || msg.contains("429") || msg.contains("too many") {
        "rate_limited"
    } else if msg.contains("connection")
        || msg.contains("dns")
        || msg.contains("network")
        || msg.contains("reset")
        || msg.contains("refused")
        || msg.contains("broken pipe")
    {
        "network"
    } else {
        "other"
    }
}

/// Helper to time an RPC call and record metrics on both success and failure.
pub async fn timed_rpc<F, T>(chain_id: u64, method: &str, fut: F) -> Result<T, EvmError>
where
    F: std::future::Future<Output = Result<T, EvmError>>,
{
    let start = Instant::now();
    let result = fut.await;
    let duration = start.elapsed();
    match &result {
        Ok(_) => record_rpc_ok(chain_id, method, duration),
        Err(e) => record_rpc_err(chain_id, method, duration, e),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::EvmError;

    #[test]
    fn test_classify_timeout() {
        let err = EvmError::Timeout("request timed out".into());
        assert_eq!(classify_error(&err), "timeout");
    }

    #[test]
    fn test_classify_rate_limited() {
        let err = EvmError::Rpc("429 Too Many Requests".into());
        assert_eq!(classify_error(&err), "rate_limited");
    }

    #[test]
    fn test_classify_network() {
        let err = EvmError::Connection("connection refused".into());
        assert_eq!(classify_error(&err), "network");
    }

    #[test]
    fn test_classify_other() {
        let err = EvmError::Rpc("invalid response".into());
        assert_eq!(classify_error(&err), "other");
    }
}
