//! Invoice cleanup service.
//!
//! Handles invoice lifecycle cleanup:
//! - Expires pending invoices that have passed their expiration time
//! - Unwatches addresses for completed invoices (expired, paid, cancelled)
//!
//! Triggered by:
//! - Block events from any chain (via EventConsumer)
//! - Periodic fallback timer

use std::pin::pin;
use std::sync::Arc;

use evm::Address;
use futures::StreamExt;
use types::{InvoiceReader, InvoiceWriter, Network, WatchedAddressReader, WatchedAddressWriter};

use super::evm_monitor::EVMMonitor;

/// Trait alias for data service requirements.
///
/// A data service must implement all repository traits needed by the cleanup service.
pub trait CleanupDataService:
    InvoiceReader + InvoiceWriter + WatchedAddressReader + WatchedAddressWriter + Send + Sync
{
}

/// Blanket implementation for any type implementing the required traits.
impl<T> CleanupDataService for T where
    T: InvoiceReader + InvoiceWriter + WatchedAddressReader + WatchedAddressWriter + Send + Sync
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
///
/// Can be triggered in two ways:
/// 1. Event-triggered: Call `check_network()` when block events arrive
/// 2. Timer-triggered: The `run()` loop checks all networks periodically
pub struct InvoiceCleanupService<D: CleanupDataService, M: EVMMonitor> {
    data_service: Arc<D>,
    evm_monitor: Arc<M>,
    config: CleanupConfig,
}

impl<D: CleanupDataService + 'static, M: EVMMonitor> InvoiceCleanupService<D, M> {
    /// Create a new invoice cleanup service.
    pub fn new(
        data_service: Arc<D>,
        evm_monitor: Arc<M>,
        config: CleanupConfig,
    ) -> Self {
        Self {
            data_service,
            evm_monitor,
            config,
        }
    }

    /// Check and expire invoices for a specific network.
    ///
    /// Called by EventConsumer when block events arrive for a network.
    /// Uses streaming to minimize memory usage.
    ///
    /// Only expires invoices with status='pending' (no payments detected).
    /// Invoices with processing or partially_paid status are left unchanged.
    pub async fn check_network(&self, network: Network) -> Result<u64, CleanupError> {
        tracing::debug!(?network, "Checking expired invoices for network");

        let mut expired_count = 0u64;
        let mut stream = pin!(InvoiceReader::stream_expired_pending_for_network(&*self.data_service, network));

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
    pub async fn check_all_networks(&self) -> Result<u64, CleanupError> {
        tracing::debug!("Checking expired invoices for all networks");

        let mut expired_count = 0u64;
        let mut stream = pin!(InvoiceReader::stream_all_expired_pending(&*self.data_service));

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
            if let Err(e) = self.unwatch_and_deactivate(&info.address, info.network).await {
                tracing::warn!(
                    address = %info.address,
                    invoice_id = %info.invoice_id,
                    error = %e,
                    "Failed to cleanup expired address"
                );
            } else {
                tracing::debug!(
                    address = %info.address,
                    invoice_id = %info.invoice_id,
                    network = ?info.network,
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
            if let Err(e) = self.unwatch_and_deactivate(&info.address, info.network).await {
                tracing::warn!(
                    address = %info.address,
                    invoice_id = %info.invoice_id,
                    error = %e,
                    "Failed to cleanup paid address"
                );
            } else {
                tracing::debug!(
                    address = %info.address,
                    invoice_id = %info.invoice_id,
                    network = ?info.network,
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
            if let Err(e) = self.unwatch_and_deactivate(&info.address, info.network).await {
                tracing::warn!(
                    address = %info.address,
                    invoice_id = %info.invoice_id,
                    error = %e,
                    "Failed to cleanup cancelled address"
                );
            } else {
                tracing::debug!(
                    address = %info.address,
                    invoice_id = %info.invoice_id,
                    network = ?info.network,
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
        network: Network,
    ) -> Result<(), CleanupError> {
        // Parse address
        let addr: Address = address.parse().map_err(|_| {
            CleanupError::InvalidAddress(address.to_string())
        })?;

        // Send UnwatchAddress command to monitor
        self.evm_monitor.unwatch_address(network, addr).await?;

        // Deactivate in database
        WatchedAddressWriter::deactivate(&*self.data_service, address, network).await?;

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

/// Error type for cleanup operations.
#[derive(Debug, thiserror::Error)]
pub enum CleanupError {
    #[error("database error: {0}")]
    Database(#[from] types::RepositoryError),

    #[error("monitor error: {0}")]
    Monitor(#[from] super::evm_monitor::EVMMonitorError),

    #[error("invalid address: {0}")]
    InvalidAddress(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use data_service::InMemoryDataService;
    use evm::U256;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use types::{InvoiceData, InvoiceId, InvoiceStatus, StoreId};
    use uuid::Uuid;

    /// Mock EVM monitor that tracks unwatch calls.
    struct MockEVMMonitor {
        unwatch_count: AtomicUsize,
    }

    impl MockEVMMonitor {
        fn new() -> Self {
            Self {
                unwatch_count: AtomicUsize::new(0),
            }
        }

        fn unwatch_count(&self) -> usize {
            self.unwatch_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl super::super::evm_monitor::EVMMonitor for MockEVMMonitor {
        async fn watch_address(
            &self,
            _network: Network,
            _address: Address,
            _invoice_id: Uuid,
            _expected_amount: Option<U256>,
            _token_contract: Option<Address>,
        ) -> Result<(), super::super::evm_monitor::EVMMonitorError> {
            Ok(())
        }

        async fn unwatch_address(
            &self,
            _network: Network,
            _address: Address,
        ) -> Result<(), super::super::evm_monitor::EVMMonitorError> {
            self.unwatch_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn health_check(&self) -> Result<(), super::super::evm_monitor::EVMMonitorError> {
            Ok(())
        }
    }

    fn create_test_invoice(status: InvoiceStatus, expired: bool) -> InvoiceData {
        let now = chrono::Utc::now();
        let expires_at = if expired {
            now - chrono::Duration::hours(1)
        } else {
            now + chrono::Duration::hours(1)
        };

        InvoiceData {
            id: InvoiceId::new(),
            store_id: StoreId(Uuid::new_v4()),
            network: Network::Ethereum,
            status,
            amount: "1000000000000000000".to_string(), // 1 ETH
            amount_received: "0".to_string(),
            asset_symbol: "ETH".to_string(),
            payment_address: Some("0x1234567890123456789012345678901234567890".to_string()),
            payment_request: None,
            created_at: now - chrono::Duration::hours(2),
            expires_at,
            metadata: None,
            extra: None,
        }
    }

    #[tokio::test]
    async fn test_cleanup_config_default() {
        let config = CleanupConfig::default();
        assert_eq!(config.fallback_interval_secs, 60);
        assert_eq!(config.unwatch_grace_period_secs, 60);
    }

    #[tokio::test]
    async fn test_cleanup_stats_total() {
        let stats = CleanupStats {
            expired: 5,
            paid: 3,
            cancelled: 2,
        };
        assert_eq!(stats.total(), 10);
    }

    #[tokio::test]
    async fn test_check_network_expires_pending_invoices() {
        let data_service = Arc::new(InMemoryDataService::new());
        let monitor = Arc::new(MockEVMMonitor::new());
        let config = CleanupConfig::default();

        // Create an expired pending invoice
        let invoice = create_test_invoice(InvoiceStatus::Pending, true);
        types::InvoiceWriter::upsert(&*data_service, &invoice)
            .await
            .unwrap();

        let service = InvoiceCleanupService::new(
            Arc::clone(&data_service),
            Arc::clone(&monitor),
            config,
        );

        // Check for expired invoices
        let expired = service.check_network(Network::Ethereum).await.unwrap();
        assert_eq!(expired, 1);

        // Verify invoice status changed to Expired
        let updated = types::InvoiceReader::get(&*data_service, &invoice.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, InvoiceStatus::Expired);
    }

    #[tokio::test]
    async fn test_check_network_ignores_non_pending_invoices() {
        let data_service = Arc::new(InMemoryDataService::new());
        let monitor = Arc::new(MockEVMMonitor::new());
        let config = CleanupConfig::default();

        // Create an expired but already paid invoice
        let invoice = create_test_invoice(InvoiceStatus::Paid, true);
        types::InvoiceWriter::upsert(&*data_service, &invoice)
            .await
            .unwrap();

        let service = InvoiceCleanupService::new(
            Arc::clone(&data_service),
            Arc::clone(&monitor),
            config,
        );

        // Check for expired invoices - should not expire paid invoices
        let expired = service.check_network(Network::Ethereum).await.unwrap();
        assert_eq!(expired, 0);

        // Verify invoice status unchanged
        let updated = types::InvoiceReader::get(&*data_service, &invoice.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, InvoiceStatus::Paid);
    }

    #[tokio::test]
    async fn test_check_network_ignores_non_expired_invoices() {
        let data_service = Arc::new(InMemoryDataService::new());
        let monitor = Arc::new(MockEVMMonitor::new());
        let config = CleanupConfig::default();

        // Create a non-expired pending invoice
        let invoice = create_test_invoice(InvoiceStatus::Pending, false);
        types::InvoiceWriter::upsert(&*data_service, &invoice)
            .await
            .unwrap();

        let service = InvoiceCleanupService::new(
            Arc::clone(&data_service),
            Arc::clone(&monitor),
            config,
        );

        // Check for expired invoices - should not expire non-expired invoices
        let expired = service.check_network(Network::Ethereum).await.unwrap();
        assert_eq!(expired, 0);
    }

    #[tokio::test]
    async fn test_cleanup_paid_addresses() {
        let data_service = Arc::new(InMemoryDataService::new());
        let monitor = Arc::new(MockEVMMonitor::new());
        let config = CleanupConfig::default();

        // Create a paid invoice with watched address
        let invoice = create_test_invoice(InvoiceStatus::Paid, false);
        types::InvoiceWriter::upsert(&*data_service, &invoice)
            .await
            .unwrap();

        // Add watched address
        types::WatchedAddressWriter::upsert(
            &*data_service,
            invoice.payment_address.as_ref().unwrap(),
            &invoice.id,
            invoice.network,
        )
        .await
        .unwrap();

        let service = InvoiceCleanupService::new(
            Arc::clone(&data_service),
            Arc::clone(&monitor),
            config,
        );

        // Cleanup addresses
        let stats = service.cleanup_addresses().await.unwrap();
        assert_eq!(stats.paid, 1);
        assert_eq!(stats.expired, 0);
        assert_eq!(stats.cancelled, 0);

        // Verify unwatch was called
        assert_eq!(monitor.unwatch_count(), 1);
    }

    #[tokio::test]
    async fn test_cleanup_cancelled_addresses() {
        let data_service = Arc::new(InMemoryDataService::new());
        let monitor = Arc::new(MockEVMMonitor::new());
        let config = CleanupConfig::default();

        // Create a cancelled invoice with watched address
        let invoice = create_test_invoice(InvoiceStatus::Cancelled, false);
        types::InvoiceWriter::upsert(&*data_service, &invoice)
            .await
            .unwrap();

        // Add watched address
        types::WatchedAddressWriter::upsert(
            &*data_service,
            invoice.payment_address.as_ref().unwrap(),
            &invoice.id,
            invoice.network,
        )
        .await
        .unwrap();

        let service = InvoiceCleanupService::new(
            Arc::clone(&data_service),
            Arc::clone(&monitor),
            config,
        );

        // Cleanup addresses
        let stats = service.cleanup_addresses().await.unwrap();
        assert_eq!(stats.cancelled, 1);
        assert_eq!(stats.paid, 0);
        assert_eq!(stats.expired, 0);

        // Verify unwatch was called
        assert_eq!(monitor.unwatch_count(), 1);
    }
}
