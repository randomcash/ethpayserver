//! Background service to retry failed WatchAddress commands.
//!
//! Periodically checks for watched addresses that haven't been notified
//! to evmmonitor and retries sending the WatchAddress command.

use std::sync::Arc;
use std::time::Duration;

use evm::{Address, U256};
use tokio::time::interval;
use types::{WatchedAddressReader, WatchedAddressWriter};
use uuid::Uuid;

use crate::services::EVMMonitor;

/// Trait for data service requirements in WatchRetryService.
pub trait WatchRetryDataService: WatchedAddressReader + WatchedAddressWriter + Send + Sync {}

impl<T> WatchRetryDataService for T where
    T: WatchedAddressReader + WatchedAddressWriter + Send + Sync
{
}

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

impl WatchRetryConfig {
    /// Load configuration from environment variables.
    ///
    /// - `WATCH_RETRY_INTERVAL_SECS` - Retry interval in seconds (default: 30)
    /// - `WATCH_RETRY_ENABLED` - Enable/disable retry service (default: true)
    pub fn from_env() -> Self {
        Self {
            retry_interval: Duration::from_secs(
                std::env::var("WATCH_RETRY_INTERVAL_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(30),
            ),
            enabled: std::env::var("WATCH_RETRY_ENABLED")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true),
        }
    }
}

/// Background service that retries failed WatchAddress commands.
pub struct WatchRetryService<D: WatchRetryDataService, E: EVMMonitor> {
    data_service: Arc<D>,
    evm_monitor: Arc<E>,
    config: WatchRetryConfig,
}

impl<D: WatchRetryDataService + 'static, E: EVMMonitor + 'static> WatchRetryService<D, E> {
    /// Create a new watch retry service.
    pub fn new(data_service: Arc<D>, evm_monitor: Arc<E>, config: WatchRetryConfig) -> Self {
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
        let pending = WatchedAddressReader::get_pending(&*self.data_service).await?;

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

            // Parse expected amount
            let expected_amount = watch
                .expected_amount
                .as_ref()
                .and_then(|s| s.parse::<U256>().ok());

            // Parse token contract if present
            let token_contract: Option<Address> =
                watch.token_address.as_ref().and_then(|s| s.parse().ok());

            // Try to send WatchAddress command using chain_id (works for both mainnets and testnets)
            match self
                .evm_monitor
                .watch_address_by_chain_id(
                    watch.chain_id,
                    address,
                    invoice_id,
                    expected_amount,
                    token_contract,
                )
                .await
            {
                Ok(()) => {
                    // Mark as notified
                    if let Err(e) = WatchedAddressWriter::mark_notified(
                        &*self.data_service,
                        &watch.address,
                        watch.chain_id,
                        watch.token_address.as_deref(),
                    )
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
                            chain_id = watch.chain_id,
                            invoice_id = %invoice_id,
                            "Successfully retried WatchAddress command"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        address = %watch.address,
                        chain_id = watch.chain_id,
                        error = %e,
                        "Failed to retry WatchAddress command, will retry later"
                    );
                }
            }
        }

        Ok(())
    }
}
