//! Background service that exports per-chain health as Prometheus gauges.
//!
//! Polls the same Redis health key the `/health/chains` handler reads, on its
//! own timer. Gauges updated only from a request handler go stale silently
//! when nobody happens to call that endpoint; polling on a schedule makes the
//! gauge a fact about the chain instead of a side effect of traffic.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::time::interval;

use crate::metrics;
use crate::services::EVMMonitor;

/// Configuration for the chain health metrics service.
#[derive(Debug, Clone)]
pub struct ChainHealthMetricsConfig {
    /// Interval between polls of the evmmonitor health key.
    pub poll_interval: Duration,
}

impl Default for ChainHealthMetricsConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(15),
        }
    }
}

impl ChainHealthMetricsConfig {
    /// Load configuration from environment variables.
    ///
    /// - `CHAIN_HEALTH_METRICS_INTERVAL_SECS` - Poll interval in seconds (default: 15)
    pub fn from_env() -> Self {
        Self {
            poll_interval: Duration::from_secs(
                std::env::var("CHAIN_HEALTH_METRICS_INTERVAL_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(15),
            ),
        }
    }
}

/// Background service that polls evmmonitor's published health data and
/// exports it as Prometheus gauges: current block, last processed block,
/// block lag, chain health, and watched addresses.
pub struct ChainHealthMetricsService<E: EVMMonitor> {
    evm_monitor: Arc<E>,
    config: ChainHealthMetricsConfig,
    /// Chain IDs seen in a prior successful poll, so a failed poll has
    /// something to mark unhealthy instead of leaving stale gauges in place.
    known_chain_ids: Mutex<HashSet<u64>>,
}

impl<E: EVMMonitor + 'static> ChainHealthMetricsService<E> {
    /// Create a new chain health metrics service.
    pub fn new(evm_monitor: Arc<E>, config: ChainHealthMetricsConfig) -> Self {
        Self {
            evm_monitor,
            config,
            known_chain_ids: Mutex::new(HashSet::new()),
        }
    }

    /// Poll evmmonitor once and update the gauges from the result.
    ///
    /// Split out from `run` so a test can drive one iteration directly
    /// instead of waiting on the timer.
    async fn poll_once(&self) {
        match self.evm_monitor.get_chain_health().await {
            Ok(chains) => {
                if let Ok(mut known) = self.known_chain_ids.lock() {
                    known.extend(chains.iter().map(|c| c.chain_id));
                }
                for chain in &chains {
                    metrics::set_chain_blocks(
                        chain.chain_id,
                        chain.current_block,
                        chain.last_processed_block,
                    );
                    metrics::set_chain_healthy(chain.chain_id, chain.is_healthy);
                    metrics::set_watched_addresses(chain.chain_id, chain.watched_addresses);
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to get chain health from Redis");
                // A poller that can't reach the health key is exactly as blind
                // as the request handler this service replaced: the gauges
                // would otherwise sit frozen at their last value, which reads
                // as healthy if that's what they last were. Mark every chain
                // we've previously reported on unhealthy rather than silent.
                if let Ok(known) = self.known_chain_ids.lock() {
                    for &chain_id in known.iter() {
                        metrics::set_chain_healthy(chain_id, false);
                    }
                }
            }
        }
    }

    /// Run the service as a background task.
    ///
    /// Should be spawned with `tokio::spawn(service.run())`.
    pub async fn run(self) {
        tracing::info!(
            interval_secs = self.config.poll_interval.as_secs(),
            "Starting chain health metrics service"
        );

        let mut interval = interval(self.config.poll_interval);

        loop {
            interval.tick().await;
            self.poll_once().await;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::VecDeque;

    use async_trait::async_trait;
    use evm::monitor::{ChainHealth, SourceStatus};
    use metrics_exporter_prometheus::PrometheusBuilder;

    use super::*;
    use crate::services::evm_monitor::EVMMonitorError;

    /// EVMMonitor that replays a scripted sequence of `get_chain_health`
    /// results, one per call, so a test can drive `poll_once` through
    /// consecutive polls without a real timer.
    struct ScriptedMonitor {
        responses: Mutex<VecDeque<Result<Vec<ChainHealth>, EVMMonitorError>>>,
    }

    impl ScriptedMonitor {
        fn new(responses: Vec<Result<Vec<ChainHealth>, EVMMonitorError>>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
            }
        }
    }

    #[async_trait]
    impl EVMMonitor for ScriptedMonitor {
        async fn watch_address(
            &self,
            _chain_id: &types::ChainId,
            _address: evm::Address,
            _invoice_id: uuid::Uuid,
            _expected_amount: Option<evm::U256>,
            _token_contract: Option<evm::Address>,
        ) -> Result<(), EVMMonitorError> {
            Ok(())
        }

        async fn watch_address_by_chain_id(
            &self,
            _chain_id: u64,
            _address: evm::Address,
            _invoice_id: uuid::Uuid,
            _expected_amount: Option<evm::U256>,
            _token_contract: Option<evm::Address>,
        ) -> Result<(), EVMMonitorError> {
            Ok(())
        }

        async fn unwatch_address(
            &self,
            _chain_id: &types::ChainId,
            _address: evm::Address,
            _token_contract: Option<evm::Address>,
        ) -> Result<(), EVMMonitorError> {
            Ok(())
        }

        async fn unwatch_address_by_chain_id(
            &self,
            _chain_id: u64,
            _address: evm::Address,
            _token_contract: Option<evm::Address>,
        ) -> Result<(), EVMMonitorError> {
            Ok(())
        }

        async fn health_check(&self) -> Result<(), EVMMonitorError> {
            Ok(())
        }

        async fn get_chain_health(&self) -> Result<Vec<ChainHealth>, EVMMonitorError> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(vec![]))
        }
    }

    fn chain_health(current: u64, last_processed: u64, healthy: bool) -> ChainHealth {
        ChainHealth {
            chain_id: 1,
            chain_name: "test".to_string(),
            status: SourceStatus::Connected,
            current_block: Some(current),
            last_processed_block: Some(last_processed),
            watched_addresses: 2,
            is_healthy: healthy,
        }
    }

    /// Runs `poll_once` on a scripted monitor with a Prometheus recorder
    /// scoped to this thread only, so parallel tests don't race over the
    /// global recorder singleton.
    fn poll_and_render<E: EVMMonitor + 'static>(service: &ChainHealthMetricsService<E>) -> String {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        ::metrics::with_local_recorder(&recorder, || rt.block_on(service.poll_once()));
        handle.render()
    }

    // This is the ablation the ticket asks for, run through the service
    // itself rather than the leaf gauge setters: a monitor that falls behind
    // must show rising lag and flip to unhealthy, and a monitor that cannot
    // be reached at all must not read as healthy just because nothing fresh
    // arrived. A chain_id swap at the call site (current vs. last_processed)
    // would pass the leaf-function tests in `metrics.rs` but fail this one.
    #[test]
    fn test_stalled_monitor_is_visible_through_the_service() {
        let monitor = Arc::new(ScriptedMonitor::new(vec![
            Ok(vec![chain_health(100, 100, true)]),
            Err(EVMMonitorError::Bridge(evm::EvmError::Monitor(
                "redis unreachable".to_string(),
            ))),
            Ok(vec![chain_health(103, 100, false)]),
        ]));
        let service = ChainHealthMetricsService::new(monitor, ChainHealthMetricsConfig::default());

        let output = poll_and_render(&service);
        assert!(output.contains("payserver_chain_current_block{chain_id=\"1\"} 100"));
        assert!(output.contains("payserver_chain_last_processed_block{chain_id=\"1\"} 100"));
        assert!(output.contains("payserver_chain_block_lag{chain_id=\"1\"} 0"));
        assert!(output.contains("payserver_chain_healthy{chain_id=\"1\"} 1"));
        assert!(output.contains("ethpayserver_watched_addresses{chain_id=\"1\"} 2"));

        // Redis becomes unreachable: a stuck poller is exactly as blind as
        // the request handler it replaced, so the chain must not keep
        // reading healthy just because nothing fresh arrived.
        let output = poll_and_render(&service);
        assert!(
            output.contains("payserver_chain_healthy{chain_id=\"1\"} 0"),
            "a failed poll left the chain reading healthy: {output}"
        );

        // The chain falls behind for real.
        let output = poll_and_render(&service);
        assert!(
            output.contains("payserver_chain_block_lag{chain_id=\"1\"} 3"),
            "lag did not climb when the monitor fell behind: {output}"
        );
        assert!(
            output.contains("payserver_chain_healthy{chain_id=\"1\"} 0"),
            "healthy gauge did not reflect the stalled chain: {output}"
        );
    }
}
