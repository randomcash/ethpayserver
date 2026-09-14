//! Background service that exports per-chain health as Prometheus gauges.
//!
//! Polls the same Redis health key the `/health/chains` handler reads, on its
//! own timer. Gauges updated only from a request handler go stale silently
//! when nobody happens to call that endpoint; polling on a schedule makes the
//! gauge a fact about the chain instead of a side effect of traffic.

use std::sync::Arc;
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
}

impl<E: EVMMonitor + 'static> ChainHealthMetricsService<E> {
    /// Create a new chain health metrics service.
    pub fn new(evm_monitor: Arc<E>, config: ChainHealthMetricsConfig) -> Self {
        Self {
            evm_monitor,
            config,
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

            match self.evm_monitor.get_chain_health().await {
                Ok(chains) => {
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
                }
            }
        }
    }
}
