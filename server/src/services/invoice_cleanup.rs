//! Invoice cleanup service.
//!
//! Handles invoice lifecycle cleanup:
//! - Expires pending invoices that have passed their expiration time
//! - Unwatches addresses for completed invoices (expired, paid, cancelled)
//! - Sends webhook notifications when invoices expire
//!
//! Triggered by:
//! - Block events from any chain (via EventConsumer)
//! - Periodic fallback timer

use std::pin::pin;
use std::sync::Arc;

use data_service::StoreWebhookReader;
use evm::Address;
use futures::StreamExt;
use types::{InvoiceReader, InvoiceWriter, WatchedAddressReader, WatchedAddressWriter};
use uuid::Uuid;

use crate::metrics;

use super::evm_monitor::EVMMonitor;
use super::webhook::{
    WebhookDataService, WebhookEventType, WebhookJob, WebhookPayload, WebhookService,
};

/// Trait alias for data service requirements.
///
/// A data service must implement all repository traits needed by the cleanup service.
pub trait CleanupDataService:
    InvoiceReader + InvoiceWriter + WatchedAddressReader + WatchedAddressWriter + StoreWebhookReader + Send + Sync
{
}

/// Blanket implementation for any type implementing the required traits.
impl<T> CleanupDataService for T where
    T: InvoiceReader + InvoiceWriter + WatchedAddressReader + WatchedAddressWriter + StoreWebhookReader + Send + Sync
{
}

/// Configuration for the invoice cleanup service.
#[derive(Debug, Clone)]
pub struct CleanupConfig {
    /// Fallback interval in seconds when no block events are received.
    pub fallback_interval_secs: u64,
    /// Grace period in seconds after invoice expires before unwatching address.
    /// This allows late payments to still be detected.
    pub unwatch_grace_period_secs: u64,
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            fallback_interval_secs: 60,
            unwatch_grace_period_secs: 60,
        }
    }
}

impl CleanupConfig {
    /// Load configuration from environment variables.
    ///
    /// - `CLEANUP_FALLBACK_INTERVAL_SECS` - Fallback check interval (default: 60)
    /// - `CLEANUP_UNWATCH_GRACE_PERIOD_SECS` - Grace period before unwatching (default: 60)
    pub fn from_env() -> Self {
        Self {
            fallback_interval_secs: std::env::var("CLEANUP_FALLBACK_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(60),
            unwatch_grace_period_secs: std::env::var("CLEANUP_UNWATCH_GRACE_PERIOD_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(60),
        }
    }
}

/// Service that handles invoice cleanup tasks.
///
/// This service:
/// 1. Expires pending invoices past their expiration time
/// 2. Unwatches addresses for expired invoices (after grace period)
/// 3. Unwatches addresses for paid invoices
/// 4. Unwatches addresses for cancelled invoices
/// 5. Sends webhook notifications when invoices expire
///
/// Can be triggered in two ways:
/// 1. Event-triggered: Call `check_expired()` when block events arrive
/// 2. Timer-triggered: The `run()` loop checks periodically
pub struct InvoiceCleanupService<D: CleanupDataService, M: EVMMonitor, W: WebhookDataService = D> {
    data_service: Arc<D>,
    evm_monitor: Arc<M>,
    config: CleanupConfig,
    webhook_service: Option<Arc<WebhookService<W>>>,
}

impl<D: CleanupDataService + 'static, M: EVMMonitor, W: WebhookDataService + 'static> InvoiceCleanupService<D, M, W> {
    /// Create a new invoice cleanup service.
    pub fn new(
        data_service: Arc<D>,
        evm_monitor: Arc<M>,
        config: CleanupConfig,
        webhook_service: Option<Arc<WebhookService<W>>>,
    ) -> Self {
        Self {
            data_service,
            evm_monitor,
            config,
            webhook_service,
        }
    }

    /// Check and expire invoices for a specific chain (triggers full check).
    ///
    /// Called by EventConsumer when block events arrive.
    /// With network-agnostic invoices, this triggers a check of all expired invoices.
    pub async fn check_chain(&self, _chain_id: u64) -> Result<u64, CleanupError> {
        self.check_expired().await
    }

    /// Check and expire all pending invoices that have passed their expiration time.
    ///
    /// Uses streaming to minimize memory usage.
    /// Only expires invoices with status='pending' (no payments detected).
    /// Invoices with processing or partially_paid status are left unchanged.
    pub async fn check_expired(&self) -> Result<u64, CleanupError> {
        tracing::debug!("Checking expired invoices");

        let mut expired_count = 0u64;
        let mut stream = pin!(InvoiceReader::stream_expired_pending(&*self.data_service));

        while let Some(result) = stream.next().await {
            match result {
                Ok(invoice_id) => {
                    match InvoiceWriter::expire(&*self.data_service, &invoice_id).await {
                        Ok(true) => {
                            expired_count += 1;
                            tracing::debug!(
                                invoice_id = %invoice_id.as_str(),
                                "Expired invoice"
                            );
                            metrics::record_invoice_expired();
                            // Queue webhook notification for expiration
                            self.queue_expiration_webhook(&invoice_id).await;
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
            tracing::info!(expired_count, "Expired pending invoices");
        }

        Ok(expired_count)
    }

    /// Queue a webhook notification for an expired invoice.
    ///
    /// This is a non-blocking operation - errors are logged but don't stop expiration processing.
    async fn queue_expiration_webhook(&self, invoice_id: &types::InvoiceId) {
        let Some(webhook_service) = &self.webhook_service else {
            return;
        };

        // Fetch the invoice to get store_id and details
        let invoice = match InvoiceReader::get(&*self.data_service, invoice_id).await {
            Ok(Some(inv)) => inv,
            Ok(None) => {
                tracing::warn!(invoice_id = %invoice_id.as_str(), "Invoice not found for webhook");
                return;
            }
            Err(e) => {
                tracing::warn!(invoice_id = %invoice_id.as_str(), error = %e, "Failed to fetch invoice for webhook");
                return;
            }
        };

        // Look up webhook config for the store
        let webhook_config = match StoreWebhookReader::get_enabled_webhook(
            &*self.data_service,
            invoice.store_id.0,
        ).await {
            Ok(Some(config)) => config,
            Ok(None) => {
                tracing::trace!(
                    store_id = %invoice.store_id.0,
                    "No webhook configured for store"
                );
                return;
            }
            Err(e) => {
                tracing::warn!(
                    store_id = %invoice.store_id.0,
                    error = %e,
                    "Failed to get webhook config"
                );
                return;
            }
        };

        // Create webhook payload
        // With network-agnostic invoices, we use the invoice currency for asset_symbol
        // and set chain_id to 0 (no specific chain for expiration events)
        let payload = WebhookPayload {
            event_id: Uuid::new_v4(),
            event_type: WebhookEventType::InvoiceExpired,
            timestamp: chrono::Utc::now(),
            invoice_id: invoice.id.as_str().to_string(),
            store_id: invoice.store_id.0,
            status: invoice.status.to_string(),
            amount: invoice.amount.clone(),
            amount_received: invoice.amount_received.clone(),
            asset_symbol: invoice.currency.clone(),
            chain_id: 0, // No specific chain for network-agnostic invoices
            network: None, // Network-agnostic
            payment: None,
        };

        // Create job and queue it
        let job = WebhookJob::new(
            webhook_config.webhook_url,
            webhook_config.webhook_secret,
            payload,
        );

        if let Err(e) = webhook_service.queue_webhook(job).await {
            tracing::warn!(
                invoice_id = %invoice.id.as_str(),
                error = %e,
                "Failed to queue expiration webhook"
            );
        }
    }

    /// Cleanup addresses for completed invoices.
    ///
    /// This method:
    /// 1. Unwatches addresses for expired invoices past grace period
    /// 2. Unwatches addresses for paid invoices
    /// 3. Unwatches addresses for cancelled invoices
    pub async fn cleanup_addresses(&self) -> Result<CleanupStats, CleanupError> {
        let mut stats = CleanupStats::default();

        // Cleanup expired invoices past grace period
        stats.expired = self.cleanup_expired_addresses().await?;

        // Cleanup paid invoices
        stats.paid = self.cleanup_paid_addresses().await?;

        // Cleanup cancelled invoices
        stats.cancelled = self.cleanup_cancelled_addresses().await?;

        if stats.total() > 0 {
            tracing::info!(
                expired = stats.expired,
                paid = stats.paid,
                cancelled = stats.cancelled,
                "Cleaned up watched addresses"
            );
        }

        Ok(stats)
    }

    /// Cleanup addresses for expired invoices past grace period.
    async fn cleanup_expired_addresses(&self) -> Result<u64, CleanupError> {
        let addresses = WatchedAddressReader::get_expired_for_cleanup(
            &*self.data_service,
            self.config.unwatch_grace_period_secs as i64,
        )
        .await?;

        let mut count = 0u64;
        for info in addresses {
            if let Err(e) = self.unwatch_and_deactivate(&info.address, info.chain_id, info.token_address.as_deref()).await {
                tracing::warn!(
                    address = %info.address,
                    invoice_id = %info.invoice_id,
                    chain_id = info.chain_id,
                    error = %e,
                    "Failed to cleanup expired address"
                );
            } else {
                tracing::debug!(
                    address = %info.address,
                    invoice_id = %info.invoice_id,
                    chain_id = info.chain_id,
                    "Unwatched expired invoice address"
                );
                count += 1;
            }
        }

        Ok(count)
    }

    /// Cleanup addresses for paid invoices.
    async fn cleanup_paid_addresses(&self) -> Result<u64, CleanupError> {
        let addresses = WatchedAddressReader::get_paid_for_cleanup(&*self.data_service).await?;

        let mut count = 0u64;
        for info in addresses {
            if let Err(e) = self.unwatch_and_deactivate(&info.address, info.chain_id, info.token_address.as_deref()).await {
                tracing::warn!(
                    address = %info.address,
                    invoice_id = %info.invoice_id,
                    chain_id = info.chain_id,
                    error = %e,
                    "Failed to cleanup paid address"
                );
            } else {
                tracing::debug!(
                    address = %info.address,
                    invoice_id = %info.invoice_id,
                    chain_id = info.chain_id,
                    "Unwatched paid invoice address"
                );
                count += 1;
            }
        }

        Ok(count)
    }

    /// Cleanup addresses for cancelled invoices.
    async fn cleanup_cancelled_addresses(&self) -> Result<u64, CleanupError> {
        let addresses = WatchedAddressReader::get_cancelled_for_cleanup(&*self.data_service).await?;

        let mut count = 0u64;
        for info in addresses {
            if let Err(e) = self.unwatch_and_deactivate(&info.address, info.chain_id, info.token_address.as_deref()).await {
                tracing::warn!(
                    address = %info.address,
                    invoice_id = %info.invoice_id,
                    chain_id = info.chain_id,
                    error = %e,
                    "Failed to cleanup cancelled address"
                );
            } else {
                tracing::debug!(
                    address = %info.address,
                    invoice_id = %info.invoice_id,
                    chain_id = info.chain_id,
                    "Unwatched cancelled invoice address"
                );
                count += 1;
            }
        }

        Ok(count)
    }

    /// Send unwatch command and deactivate address in database.
    async fn unwatch_and_deactivate(
        &self,
        address: &str,
        chain_id: u64,
        token_address: Option<&str>,
    ) -> Result<(), CleanupError> {
        // Parse address
        let addr: Address = address.parse().map_err(|_| {
            CleanupError::InvalidAddress(address.to_string())
        })?;

        // Parse token contract address
        let token_contract: Option<Address> = token_address.and_then(|t| t.parse().ok());

        // Send UnwatchAddress command to monitor (using chain_id for testnet support)
        self.evm_monitor.unwatch_address_by_chain_id(chain_id, addr, token_contract).await?;

        // Deactivate in database
        WatchedAddressWriter::deactivate(&*self.data_service, address, chain_id, token_address).await?;

        Ok(())
    }

    /// Run the cleanup service as a background task.
    ///
    /// This runs a periodic timer that:
    /// 1. Expires pending invoices
    /// 2. Cleans up watched addresses for completed invoices
    ///
    /// Should be spawned with `tokio::spawn(service.run())`.
    pub async fn run(self: Arc<Self>) {
        tracing::info!(
            interval_secs = self.config.fallback_interval_secs,
            grace_period_secs = self.config.unwatch_grace_period_secs,
            "Starting invoice cleanup service"
        );

        let mut interval = tokio::time::interval(
            std::time::Duration::from_secs(self.config.fallback_interval_secs)
        );

        loop {
            interval.tick().await;

            // Expire pending invoices
            match self.check_expired().await {
                Ok(count) => {
                    if count > 0 {
                        tracing::info!(expired_count = count, "Expired invoices");
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to expire invoices");
                }
            }

            // Cleanup watched addresses
            match self.cleanup_addresses().await {
                Ok(stats) => {
                    if stats.total() > 0 {
                        tracing::debug!(
                            expired = stats.expired,
                            paid = stats.paid,
                            cancelled = stats.cancelled,
                            "Address cleanup complete"
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to cleanup addresses");
                }
            }
        }
    }
}

/// Statistics from address cleanup.
#[derive(Debug, Default)]
pub struct CleanupStats {
    /// Number of addresses cleaned up for expired invoices.
    pub expired: u64,
    /// Number of addresses cleaned up for paid invoices.
    pub paid: u64,
    /// Number of addresses cleaned up for cancelled invoices.
    pub cancelled: u64,
}

impl CleanupStats {
    /// Total number of addresses cleaned up.
    pub fn total(&self) -> u64 {
        self.expired + self.paid + self.cancelled
    }
}

/// Errors that can occur during cleanup operations.
#[derive(Debug, thiserror::Error)]
pub enum CleanupError {
    #[error("Repository error: {0}")]
    Repository(#[from] types::RepositoryError),

    #[error("Monitor error: {0}")]
    Monitor(String),

    #[error("Invalid address: {0}")]
    InvalidAddress(String),
}

impl From<Box<dyn std::error::Error + Send + Sync>> for CleanupError {
    fn from(e: Box<dyn std::error::Error + Send + Sync>) -> Self {
        CleanupError::Monitor(e.to_string())
    }
}

impl From<super::evm_monitor::EVMMonitorError> for CleanupError {
    fn from(e: super::evm_monitor::EVMMonitorError) -> Self {
        CleanupError::Monitor(e.to_string())
    }
}
