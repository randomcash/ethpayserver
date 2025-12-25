//! Invoice expiration service.
//!
//! Expires pending invoices that have passed their expiration time.
//! Triggered by:
//! - Block events from any chain (via EventConsumer)
//! - 60-second fallback timer

use std::pin::pin;
use std::sync::Arc;

use data_service::PgDataService;
use futures::StreamExt;
use types::{ExpiredInvoiceStreamer, InvoiceWriter, Network};

/// Configuration for the invoice expiration service.
#[derive(Debug, Clone)]
pub struct ExpirationConfig {
    /// Fallback interval in seconds when no block events are received.
    pub fallback_interval_secs: u64,
}

impl Default for ExpirationConfig {
    fn default() -> Self {
        Self {
            fallback_interval_secs: 60,
        }
    }
}

/// Service that expires pending invoices past their expiration time.
///
/// This service can be triggered in two ways:
/// 1. Event-triggered: Call `check_network()` when block events arrive
/// 2. Timer-triggered: The `run()` loop checks all networks every 60 seconds
pub struct InvoiceExpirationService {
    data_service: Arc<PgDataService>,
    config: ExpirationConfig,
}

impl InvoiceExpirationService {
    /// Create a new invoice expiration service.
    pub fn new(data_service: Arc<PgDataService>, config: ExpirationConfig) -> Self {
        Self { data_service, config }
    }

    /// Check and expire invoices for a specific network.
    ///
    /// Called by EventConsumer when block events arrive for a network.
    /// Uses streaming to minimize memory usage.
    ///
    /// Only expires invoices with status='pending' (no payments detected).
    /// Invoices with processing or partially_paid status are left unchanged.
    pub async fn check_network(&self, network: Network) -> Result<u64, ExpirationError> {
        tracing::debug!(?network, "Checking expired invoices for network");

        let mut expired_count = 0u64;
        let mut stream = pin!(self.data_service.stream_expired_pending_for_network(network));

        while let Some(result) = stream.next().await {
            match result {
                Ok(invoice_id) => {
                    match InvoiceWriter::expire(&*self.data_service, &invoice_id).await {
                        Ok(true) => {
                            expired_count += 1;
                            tracing::debug!(
                                invoice_id = %invoice_id.as_str(),
                                ?network,
                                "Expired invoice"
                            );
                        }
                        Ok(false) => {
                            // Invoice was already expired or status changed
                            tracing::trace!(
                                invoice_id = %invoice_id.as_str(),
                                "Invoice already processed"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                invoice_id = %invoice_id.as_str(),
                                error = %e,
                                "Failed to expire invoice"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Error streaming expired invoice");
                }
            }
        }

        if expired_count > 0 {
            tracing::info!(
                ?network,
                expired_count,
                "Expired pending invoices for network"
            );
        }

        Ok(expired_count)
    }

    /// Check and expire invoices for all networks.
    ///
    /// Called by the fallback timer.
    /// Uses streaming to minimize memory usage.
    pub async fn check_all_networks(&self) -> Result<u64, ExpirationError> {
        tracing::debug!("Checking expired invoices for all networks");

        let mut expired_count = 0u64;
        let mut stream = pin!(self.data_service.stream_all_expired_pending());

        while let Some(result) = stream.next().await {
            match result {
                Ok((network, invoice_id)) => {
                    match InvoiceWriter::expire(&*self.data_service, &invoice_id).await {
                        Ok(true) => {
                            expired_count += 1;
                            tracing::debug!(
                                invoice_id = %invoice_id.as_str(),
                                ?network,
                                "Expired invoice"
                            );
                        }
                        Ok(false) => {
                            tracing::trace!(
                                invoice_id = %invoice_id.as_str(),
                                "Invoice already processed"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                invoice_id = %invoice_id.as_str(),
                                error = %e,
                                "Failed to expire invoice"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Error streaming expired invoice");
                }
            }
        }

        Ok(expired_count)
    }

    /// Run the expiration service as a background task.
    ///
    /// This runs a 60-second fallback timer that checks all networks.
    /// Should be spawned with `tokio::spawn(service.run())`.
    pub async fn run(self: Arc<Self>) {
        tracing::info!(
            interval_secs = self.config.fallback_interval_secs,
            "Starting invoice expiration service"
        );

        let mut interval = tokio::time::interval(
            std::time::Duration::from_secs(self.config.fallback_interval_secs)
        );

        loop {
            interval.tick().await;

            match self.check_all_networks().await {
                Ok(count) => {
                    if count > 0 {
                        tracing::info!(expired_count = count, "Expired invoices");
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to expire invoices");
                }
            }
        }
    }
}

/// Error type for expiration operations.
#[derive(Debug, thiserror::Error)]
pub enum ExpirationError {
    #[error("database error: {0}")]
    Database(#[from] types::RepositoryError),
}
