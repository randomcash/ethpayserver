//! Background service to retry failed WatchAddress commands.
//!
//! Periodically checks for watched addresses that haven't been notified
//! to evmmonitor and retries sending the WatchAddress command.

use std::sync::Arc;
use std::time::Duration;

use data_service::PgDataService;
use evm::{network_to_chain_id, Address, U256};
use tokio::time::interval;
use uuid::Uuid;

use crate::services::EVMMonitor;

/// Configuration for the watch retry service.
#[derive(Debug, Clone)]
pub struct WatchRetryConfig {
    /// Interval between retry attempts.
    pub retry_interval: Duration,
    /// Whether the service is enabled.
    pub enabled: bool,
}

impl Default for WatchRetryConfig {
    fn default() -> Self {
        Self {
            retry_interval: Duration::from_secs(30),
            enabled: true,
        }
    }
}

/// Background service that retries failed WatchAddress commands.
pub struct WatchRetryService {
    data_service: Arc<PgDataService>,
    evm_monitor: Arc<dyn EVMMonitor>,
    config: WatchRetryConfig,
}

impl WatchRetryService {
    /// Create a new watch retry service.
    pub fn new(
        data_service: Arc<PgDataService>,
        evm_monitor: Arc<dyn EVMMonitor>,
        config: WatchRetryConfig,
    ) -> Self {
        Self {
            data_service,
            evm_monitor,
            config,
        }
    }

    /// Run the retry service as a background task.
    ///
    /// This should be spawned as a tokio task.
    pub async fn run(self) {
        if !self.config.enabled {
            tracing::info!("Watch retry service disabled");
            return;
        }

        tracing::info!(
            interval_secs = self.config.retry_interval.as_secs(),
            "Starting watch retry service"
        );

        let mut interval = interval(self.config.retry_interval);

        loop {
            interval.tick().await;

            if let Err(e) = self.retry_pending_watches().await {
                tracing::error!(error = %e, "Failed to retry pending watches");
            }
        }
    }

    /// Retry all pending watched addresses.
    async fn retry_pending_watches(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let pending = self.data_service.get_pending_watches().await?;

        if pending.is_empty() {
            return Ok(());
        }

        tracing::debug!(count = pending.len(), "Found pending watches to retry");

        for watch in pending {
            // Parse address
            let address: Address = match watch.address.parse() {
                Ok(addr) => addr,
                Err(e) => {
                    tracing::error!(
                        address = %watch.address,
                        error = %e,
                        "Failed to parse watch address, skipping"
                    );
                    continue;
                }
            };

            // Parse invoice ID as UUID
            let invoice_id: Uuid = match Uuid::parse_str(&watch.invoice_id) {
                Ok(id) => id,
                Err(e) => {
                    tracing::error!(
                        invoice_id = %watch.invoice_id,
                        error = %e,
                        "Failed to parse invoice ID, skipping"
                    );
                    continue;
                }
            };

            // Check if network is supported
            let chain_id = match network_to_chain_id(watch.network) {
                Some(id) => id,
                None => {
                    tracing::warn!(
                        network = ?watch.network,
                        "Unsupported network for watch retry, skipping"
                    );
                    continue;
                }
            };

            // Parse expected amount
            let expected_amount = watch
                .expected_amount
                .as_ref()
                .and_then(|s| s.parse::<U256>().ok());

            // Parse token contract if present
            let token_contract: Option<Address> = watch
                .token_address
                .as_ref()
                .and_then(|s| s.parse().ok());

            // Try to send WatchAddress command
            match self
                .evm_monitor
                .watch_address(watch.network, address, invoice_id, expected_amount, token_contract)
                .await
            {
                Ok(()) => {
                    // Mark as notified
                    if let Err(e) = self
                        .data_service
                        .mark_watch_notified(&watch.address, watch.network)
                        .await
                    {
                        tracing::error!(
                            address = %watch.address,
                            error = %e,
                            "Failed to mark watch as notified"
                        );
                    } else {
                        tracing::info!(
                            address = %watch.address,
                            chain_id,
                            invoice_id = %invoice_id,
                            "Successfully retried WatchAddress command"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        address = %watch.address,
                        chain_id,
                        error = %e,
                        "Failed to retry WatchAddress command, will retry later"
                    );
                }
            }
        }

        Ok(())
    }
}
