//! Event consumer service for processing monitor events.
//!
//! Subscribes to evmmonitor events via the EventBridge and updates
//! invoice/payment state in the database.

use std::sync::Arc;

use data_service::StoreWebhookReader;
use evm::chain_id_to_network;
use evm::monitor::bridge::EventBridge;
use evm::monitor::events::{MonitorEvent, PaymentConfirmed, PaymentDetected, ReorgDetected};
use rust_decimal::Decimal;
use tokio_stream::StreamExt;
use types::{
    AssetType, InvoiceData, InvoiceId, InvoiceReader, InvoiceStatus, InvoiceWriter,
    PaymentData, PaymentReader, PaymentWriter, TokenReader,
};
use uuid::Uuid;

use super::evm_monitor::EVMMonitor;
use super::invoice_cleanup::{CleanupDataService, InvoiceCleanupService};
use super::webhook::{
    WebhookDataService, WebhookEventType, WebhookJob, WebhookPayload, WebhookPaymentInfo,
    WebhookService,
};

/// Trait for data service requirements in EventConsumer.
pub trait EventConsumerDataService:
    InvoiceReader
    + InvoiceWriter
    + PaymentReader
    + PaymentWriter
    + TokenReader
    + StoreWebhookReader
    + CleanupDataService
    + Send
    + Sync
{
}

impl<T> EventConsumerDataService for T where
    T: InvoiceReader
        + InvoiceWriter
        + PaymentReader
        + PaymentWriter
        + TokenReader
        + StoreWebhookReader
        + CleanupDataService
        + Send
        + Sync
{
}

/// Event consumer that processes monitor events and updates database state.
///
/// Optionally sends webhook notifications when invoice status changes.
pub struct EventConsumer<D: EventConsumerDataService, M: EVMMonitor, W: WebhookDataService = D> {
    bridge: Arc<dyn EventBridge>,
    data_service: Arc<D>,
    cleanup_service: Option<Arc<InvoiceCleanupService<D, M, W>>>,
    webhook_service: Option<Arc<WebhookService<W>>>,
}

impl<D: EventConsumerDataService + 'static, M: EVMMonitor + 'static, W: WebhookDataService + 'static> EventConsumer<D, M, W> {
    /// Create a new event consumer with optional services.
    pub fn new(
        bridge: Arc<dyn EventBridge>,
        data_service: Arc<D>,
        cleanup_service: Option<Arc<InvoiceCleanupService<D, M, W>>>,
        webhook_service: Option<Arc<WebhookService<W>>>,
    ) -> Self {
        Self {
            bridge,
            data_service,
            cleanup_service,
            webhook_service,
        }
    }

    /// Run the event consumer as a background task.
    ///
    /// This should be spawned with `tokio::spawn(consumer.run())`.
    pub async fn run(self) {
        tracing::info!("Starting event consumer");

        let mut event_stream = match self.bridge.subscribe().await {
            Ok(stream) => stream,
            Err(e) => {
                tracing::error!(error = %e, "Failed to subscribe to events");
                return;
            }
        };

        while let Some(event) = event_stream.next().await {
            if let Err(e) = self.handle_event(event).await {
                tracing::error!(error = %e, "Failed to handle event");
            }
        }

        tracing::warn!("Event stream ended, consumer shutting down");
    }

    /// Handle a single monitor event.
    async fn handle_event(&self, event: MonitorEvent) -> Result<(), EventConsumerError> {
        match event {
            MonitorEvent::PaymentDetected(payment) => {
                self.handle_payment_detected(payment).await
            }
            MonitorEvent::PaymentConfirmed(payment) => {
                self.handle_payment_confirmed(payment).await
            }
            MonitorEvent::ReorgDetected(reorg) => {
                self.handle_reorg_detected(reorg).await
            }
            // Log other events but don't process them
            MonitorEvent::MonitorStarted { chain_id } => {
                tracing::info!(chain_id, "Monitor started");
                Ok(())
            }
            MonitorEvent::MonitorStopped { chain_id } => {
                tracing::info!(chain_id, "Monitor stopped");
                Ok(())
            }
            MonitorEvent::MonitorError { chain_id, error } => {
                tracing::warn!(chain_id, error, "Monitor error");
                Ok(())
            }
            MonitorEvent::AddressWatched(info) => {
                tracing::debug!(
                    chain_id = info.chain_id,
                    address = %info.address,
                    invoice_id = %info.invoice_id,
                    "Address watched"
                );
                Ok(())
            }
            MonitorEvent::AddressUnwatched(info) => {
                tracing::debug!(
                    chain_id = info.chain_id,
                    address = %info.address,
                    "Address unwatched"
                );
                Ok(())
            }
            MonitorEvent::StatusReport(report) => {
                tracing::debug!(
                    chain_id = report.chain_id,
                    watched_count = report.watched_count,
                    current_block = report.current_block,
                    "Status report"
                );
                // Trigger invoice expiration check for this network
                self.trigger_expiration_check(report.chain_id).await;
                Ok(())
            }
        }
    }

    /// Handle PaymentDetected event.
    ///
    /// Creates a payment record in the database. The DB trigger automatically:
    /// - Updates invoice.amount_received
    /// - Transitions invoice status: pending → processing
    async fn handle_payment_detected(&self, event: PaymentDetected) -> Result<(), EventConsumerError> {
        let network = chain_id_to_network(event.chain_id)
            .ok_or_else(|| EventConsumerError::InvalidData(
                format!("Unknown chain_id: {}", event.chain_id)
            ))?;

        // Determine asset type and symbol based on whether it's native or token
        let (asset_type, asset_symbol, token_address) = if event.is_native {
            (AssetType::Native, network_native_symbol(network), None)
        } else {
            // Look up the token symbol from the database
            let token_addr = event.token_address.ok_or_else(|| {
                EventConsumerError::InvalidData(
                    "ERC20 payment missing token_address".to_string()
                )
            })?;
            let token_addr_str = format!("{:#x}", token_addr);

            let symbol = match TokenReader::get_by_address(&*self.data_service, network, &token_addr_str).await? {
                Some(token) => token.symbol.unwrap_or_else(|| "ERC20".to_string()),
                None => {
                    tracing::warn!(
                        token_address = %token_addr_str,
                        network = ?network,
                        "Unknown token, using address as symbol"
                    );
                    // Use shortened address as fallback
                    format!("0x{}...", &token_addr_str[2..8])
                }
            };

            (AssetType::ERC20, symbol, Some(token_addr_str))
        };

        let payment = PaymentData {
            id: Uuid::new_v4(),
            invoice_id: InvoiceId::from_string(event.invoice_id.to_string()),
            network,
            asset_type,
            amount: event.amount.to_string(),
            asset_symbol,
            token_address,
            tx_hash: format!("{:#x}", event.tx_hash),
            block_number: Some(event.block_number),
            detected_at: event.detected_at,
            confirmed_at: None,
            from_address: Some(format!("{:#x}", event.from_address)),
            reorged: false,
            extra: None,
        };

        tracing::info!(
            invoice_id = %event.invoice_id,
            tx_hash = %payment.tx_hash,
            amount = %payment.amount,
            network = ?network,
            asset_type = ?asset_type,
            block_number = event.block_number,
            "Payment detected"
        );

        PaymentWriter::upsert(&*self.data_service, &payment).await?;

        // Queue webhook notification
        let invoice_id = InvoiceId::from_string(event.invoice_id.to_string());
        if let Ok(Some(invoice)) = InvoiceReader::get(&*self.data_service, &invoice_id).await {
            self.queue_webhook(WebhookEventType::PaymentDetected, &invoice, Some(&payment)).await;
        }

        Ok(())
    }

    /// Handle PaymentConfirmed event.
    ///
    /// Updates payment confirmation status and transitions invoice to `paid`
    /// if amount_received >= amount.
    async fn handle_payment_confirmed(&self, event: PaymentConfirmed) -> Result<(), EventConsumerError> {
        let invoice_id = InvoiceId::from_string(event.invoice_id.to_string());
        let tx_hash = format!("{:#x}", event.tx_hash);

        // Find the payment by invoice_id + tx_hash (only non-reorged payments)
        let payments = PaymentReader::get_valid_for_invoice(&*self.data_service, &invoice_id).await?;
        let payment = match payments.iter().find(|p| p.tx_hash == tx_hash) {
            Some(p) => p,
            None => {
                // Payment not found or was reorged - log and skip
                tracing::debug!(
                    invoice_id = %event.invoice_id,
                    tx_hash = %tx_hash,
                    "Payment not found or reorged, skipping confirmation"
                );
                return Ok(());
            }
        };

        // Mark payment as confirmed
        PaymentWriter::mark_confirmed(
            &*self.data_service,
            payment.id,
            event.confirmed_at,
        ).await?;

        tracing::info!(
            invoice_id = %event.invoice_id,
            tx_hash = %tx_hash,
            "Payment confirmed"
        );

        // Check if invoice is fully paid
        let invoice = InvoiceReader::get(&*self.data_service, &invoice_id)
            .await?
            .ok_or_else(|| EventConsumerError::InvalidData(
                format!("Invoice not found: {}", event.invoice_id)
            ))?;

        // Compare amounts using rust_decimal
        let amount_received: Decimal = invoice.amount_received.parse()
            .map_err(|e| EventConsumerError::InvalidData(
                format!("Invalid amount_received '{}': {}", invoice.amount_received, e)
            ))?;
        let amount_expected: Decimal = invoice.amount.parse()
            .map_err(|e| EventConsumerError::InvalidData(
                format!("Invalid amount '{}': {}", invoice.amount, e)
            ))?;

        let is_fully_paid = amount_received >= amount_expected;

        // Handle based on invoice status
        match invoice.status {
            InvoiceStatus::Processing | InvoiceStatus::PartiallyPaid => {
                // Normal flow: transition to paid if fully paid
                if is_fully_paid {
                    InvoiceWriter::update_status(&*self.data_service, &invoice_id, InvoiceStatus::Paid).await?;
                    tracing::info!(
                        invoice_id = %event.invoice_id,
                        amount_received = %amount_received,
                        amount_expected = %amount_expected,
                        "Invoice fully paid"
                    );

                    // Queue webhook notification for payment confirmed
                    if let Ok(Some(updated_invoice)) = InvoiceReader::get(&*self.data_service, &invoice_id).await {
                        self.queue_webhook(WebhookEventType::PaymentConfirmed, &updated_invoice, Some(payment)).await;
                    }
                }
            }
            InvoiceStatus::Expired => {
                // Late payment: invoice expired but payment still came through
                // Transition to LatePaid for merchant review
                if is_fully_paid {
                    InvoiceWriter::update_status(&*self.data_service, &invoice_id, InvoiceStatus::LatePaid).await?;
                    tracing::warn!(
                        invoice_id = %event.invoice_id,
                        amount_received = %amount_received,
                        amount_expected = %amount_expected,
                        "Late payment received on expired invoice - requires merchant review"
                    );

                    // Queue webhook notification for late payment
                    if let Ok(Some(updated_invoice)) = InvoiceReader::get(&*self.data_service, &invoice_id).await {
                        self.queue_webhook(WebhookEventType::LatePaid, &updated_invoice, Some(payment)).await;
                    }
                }
            }
            _ => {
                // Cancelled, Refunded, LatePaid, or already Paid - don't modify
                tracing::debug!(
                    invoice_id = %event.invoice_id,
                    status = ?invoice.status,
                    "Invoice in final state, skipping status update"
                );
            }
        }

        Ok(())
    }

    /// Trigger invoice expiration check for a network.
    ///
    /// Called when block events are received. This is a non-blocking operation -
    /// errors are logged but don't stop event processing.
    async fn trigger_expiration_check(&self, chain_id: u64) {
        let Some(cleanup_service) = &self.cleanup_service else {
            return;
        };

        let Some(network) = chain_id_to_network(chain_id) else {
            tracing::trace!(chain_id, "Unknown chain_id for expiration check");
            return;
        };

        match cleanup_service.check_network(network).await {
            Ok(count) => {
                if count > 0 {
                    tracing::debug!(
                        ?network,
                        expired_count = count,
                        "Expired invoices on block event"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    ?network,
                    error = %e,
                    "Failed to check expired invoices"
                );
            }
        }
    }

    /// Queue a webhook notification for an invoice status change.
    ///
    /// This is a non-blocking operation - errors are logged but don't stop event processing.
    async fn queue_webhook(
        &self,
        event_type: WebhookEventType,
        invoice: &InvoiceData,
        payment: Option<&PaymentData>,
    ) {
        let Some(webhook_service) = &self.webhook_service else {
            return;
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
        let payload = WebhookPayload {
            event_id: Uuid::new_v4(),
            event_type,
            timestamp: chrono::Utc::now(),
            invoice_id: invoice.id.as_str().to_string(),
            store_id: invoice.store_id.0,
            status: invoice.status.to_string(),
            amount: invoice.amount.clone(),
            amount_received: invoice.amount_received.clone(),
            asset_symbol: invoice.asset_symbol.clone(),
            network: invoice.network.to_string(),
            payment: payment.map(|p| WebhookPaymentInfo {
                tx_hash: p.tx_hash.clone(),
                from_address: p.from_address.clone(),
                block_number: p.block_number,
                confirmed: p.confirmed_at.is_some(),
            }),
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
                "Failed to queue webhook"
            );
        }
    }

    /// Handle ReorgDetected event.
    ///
    /// Marks affected payments as reorged and reverts invoice status:
    /// - If other valid payments exist → `processing`
    /// - If no valid payments → `pending`
    async fn handle_reorg_detected(&self, event: ReorgDetected) -> Result<(), EventConsumerError> {
        let network = chain_id_to_network(event.chain_id)
            .ok_or_else(|| EventConsumerError::InvalidData(
                format!("Unknown chain_id: {}", event.chain_id)
            ))?;

        tracing::warn!(
            chain_id = event.chain_id,
            network = ?network,
            fork_block = event.fork_block,
            depth = event.depth,
            affected_invoices = event.affected_invoices.len(),
            "Chain reorganization detected"
        );

        for invoice_uuid in &event.affected_invoices {
            let invoice_id = InvoiceId::from_string(invoice_uuid.to_string());

            // Mark payments from this chain at or after the fork block as reorged
            let reorged_count = self.data_service
                .mark_reorged(&invoice_id, network, event.fork_block)
                .await?;

            if reorged_count == 0 {
                tracing::debug!(
                    invoice_id = %invoice_uuid,
                    fork_block = event.fork_block,
                    "No payments affected by reorg"
                );
                continue;
            }

            tracing::info!(
                invoice_id = %invoice_uuid,
                reorged_count,
                fork_block = event.fork_block,
                "Marked payments as reorged"
            );

            // Determine new invoice status based on remaining valid payments
            let has_valid = PaymentReader::has_valid_payments(&*self.data_service, &invoice_id).await?;
            let new_status = if has_valid {
                InvoiceStatus::Processing
            } else {
                InvoiceStatus::Pending
            };

            InvoiceWriter::update_status(&*self.data_service, &invoice_id, new_status).await?;

            tracing::info!(
                invoice_id = %invoice_uuid,
                new_status = ?new_status,
                "Reverted invoice status after reorg"
            );
        }

        Ok(())
    }
}

/// Error type for event consumer operations.
#[derive(Debug, thiserror::Error)]
pub enum EventConsumerError {
    #[error("database error: {0}")]
    Database(#[from] types::RepositoryError),

    #[error("invalid data: {0}")]
    InvalidData(String),
}

/// Get the native asset symbol for a network.
fn network_native_symbol(network: types::Network) -> String {
    use types::Network::*;
    match network {
        Ethereum | Arbitrum | Optimism | Base | ZkSync | Linea | Scroll => "ETH",
        Polygon => "POL",
        Avalanche => "AVAX",
        BinanceSmartChain => "BNB",
        // Non-EVM networks - shouldn't reach here
        _ => "UNKNOWN",
    }.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::Utc;
    use data_service::InMemoryDataService;
    use evm::monitor::bridge::MemoryBridge;
    use evm::{Address, B256, U256};
    use std::sync::Arc;
    use types::{InvoiceStatus, Network, StoreId};

    use super::super::evm_monitor::{EVMMonitor, EVMMonitorError};

    /// Mock EVMMonitor for testing.
    struct MockEVMMonitor;

    #[async_trait]
    impl EVMMonitor for MockEVMMonitor {
        async fn watch_address(
            &self,
            _network: Network,
            _address: Address,
            _invoice_id: Uuid,
            _expected_amount: Option<U256>,
            _token_contract: Option<Address>,
        ) -> Result<(), EVMMonitorError> {
            Ok(())
        }

        async fn unwatch_address(
            &self,
            _network: Network,
            _address: Address,
        ) -> Result<(), EVMMonitorError> {
            Ok(())
        }

        async fn health_check(&self) -> Result<(), EVMMonitorError> {
            Ok(())
        }
    }

    /// Create a test invoice in the data service.
    async fn create_test_invoice(
        ds: &InMemoryDataService,
        invoice_id: &InvoiceId,
        store_id: StoreId,
    ) {
        let invoice = InvoiceData {
            id: invoice_id.clone(),
            store_id,
            network: Network::Ethereum,
            status: InvoiceStatus::Pending,
            amount: "1000000000000000000".to_string(), // 1 ETH
            amount_received: "0".to_string(),
            asset_symbol: "ETH".to_string(),
            payment_address: Some("0x1234567890abcdef1234567890abcdef12345678".to_string()),
            payment_request: None,
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            metadata: None,
            extra: None,
        };
        InvoiceWriter::upsert(ds, &invoice).await.unwrap();
    }

    #[test]
    fn test_network_native_symbol() {
        assert_eq!(network_native_symbol(Network::Ethereum), "ETH");
        assert_eq!(network_native_symbol(Network::Polygon), "POL");
        assert_eq!(network_native_symbol(Network::Avalanche), "AVAX");
        assert_eq!(network_native_symbol(Network::BinanceSmartChain), "BNB");
        assert_eq!(network_native_symbol(Network::Arbitrum), "ETH");
        assert_eq!(network_native_symbol(Network::Optimism), "ETH");
        assert_eq!(network_native_symbol(Network::Base), "ETH");
        // Non-EVM networks
        assert_eq!(network_native_symbol(Network::BitcoinMainnet), "UNKNOWN");
    }

    #[tokio::test]
    async fn test_handle_payment_detected_native() {
        let ds = Arc::new(InMemoryDataService::new());
        let bridge = Arc::new(MemoryBridge::new());

        let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> = EventConsumer::new(
            bridge.clone(),
            ds.clone(),
            None,
            None,
        );

        let invoice_id = InvoiceId::new();
        let store_id = StoreId::new();
        create_test_invoice(&*ds, &invoice_id, store_id).await;

        // Create PaymentDetected event
        let event = PaymentDetected {
            chain_id: 1, // Ethereum mainnet
            invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
            payment_address: Address::ZERO,
            amount: U256::from(500000000000000000u64), // 0.5 ETH
            tx_hash: B256::ZERO,
            block_number: 12345678,
            block_hash: B256::ZERO,
            log_index: None,
            is_native: true,
            token_address: None,
            from_address: Address::repeat_byte(0xab),
            confirmations: 1,
            required_confirmations: 12,
            detected_at: Utc::now(),
        };

        // Handle the event
        consumer.handle_payment_detected(event).await.unwrap();

        // Verify payment was created
        let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id).await.unwrap();
        assert_eq!(payments.len(), 1);
        assert_eq!(payments[0].asset_symbol, "ETH");
        assert_eq!(payments[0].amount, "500000000000000000");
        assert!(!payments[0].reorged);
    }

    #[tokio::test]
    async fn test_handle_payment_detected_unknown_chain() {
        let ds = Arc::new(InMemoryDataService::new());
        let bridge = Arc::new(MemoryBridge::new());

        let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> = EventConsumer::new(
            bridge.clone(),
            ds.clone(),
            None,
            None,
        );

        let invoice_id = InvoiceId::new();

        // Create PaymentDetected event with unknown chain
        let event = PaymentDetected {
            chain_id: 99999, // Unknown chain
            invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
            payment_address: Address::ZERO,
            amount: U256::from(1000000u64),
            tx_hash: B256::ZERO,
            block_number: 12345678,
            block_hash: B256::ZERO,
            log_index: None,
            is_native: true,
            token_address: None,
            from_address: Address::ZERO,
            confirmations: 1,
            required_confirmations: 12,
            detected_at: Utc::now(),
        };

        // Should error with invalid data
        let result = consumer.handle_payment_detected(event).await;
        assert!(matches!(result, Err(EventConsumerError::InvalidData(_))));
    }

    #[tokio::test]
    async fn test_handle_payment_confirmed_transitions_to_paid() {
        let ds = Arc::new(InMemoryDataService::new());
        let bridge = Arc::new(MemoryBridge::new());

        let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> = EventConsumer::new(
            bridge.clone(),
            ds.clone(),
            None,
            None,
        );

        let invoice_id = InvoiceId::new();
        let store_id = StoreId::new();

        // Create invoice in processing state with full amount received
        let invoice = InvoiceData {
            id: invoice_id.clone(),
            store_id,
            network: Network::Ethereum,
            status: InvoiceStatus::Processing,
            amount: "1000000000000000000".to_string(), // 1 ETH
            amount_received: "1000000000000000000".to_string(), // 1 ETH received
            asset_symbol: "ETH".to_string(),
            payment_address: Some("0x1234567890abcdef1234567890abcdef12345678".to_string()),
            payment_request: None,
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            metadata: None,
            extra: None,
        };
        InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

        // Create a payment record
        let tx_hash = B256::repeat_byte(0xab);
        let payment = PaymentData {
            id: Uuid::new_v4(),
            invoice_id: invoice_id.clone(),
            network: Network::Ethereum,
            asset_type: types::AssetType::Native,
            amount: "1000000000000000000".to_string(),
            asset_symbol: "ETH".to_string(),
            token_address: None,
            tx_hash: format!("{:#x}", tx_hash),
            block_number: Some(12345678),
            detected_at: Utc::now(),
            confirmed_at: None,
            from_address: Some("0xabababababababababababababababababababab".to_string()),
            reorged: false,
            extra: None,
        };
        PaymentWriter::upsert(&*ds, &payment).await.unwrap();

        // Create PaymentConfirmed event
        let event = PaymentConfirmed {
            chain_id: 1,
            invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
            payment_address: Address::ZERO,
            amount: U256::from(1000000000000000000u64),
            tx_hash,
            block_number: 12345678,
            confirmations: 12,
            confirmed_at: Utc::now(),
        };

        // Handle the event
        consumer.handle_payment_confirmed(event).await.unwrap();

        // Verify invoice was marked as paid
        let invoice = InvoiceReader::get(&*ds, &invoice_id).await.unwrap().unwrap();
        assert_eq!(invoice.status, InvoiceStatus::Paid);

        // Verify payment was marked as confirmed
        let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id).await.unwrap();
        assert!(payments[0].confirmed_at.is_some());
    }

    #[tokio::test]
    async fn test_handle_payment_confirmed_skips_cancelled_invoice() {
        let ds = Arc::new(InMemoryDataService::new());
        let bridge = Arc::new(MemoryBridge::new());

        let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> = EventConsumer::new(
            bridge.clone(),
            ds.clone(),
            None,
            None,
        );

        let invoice_id = InvoiceId::new();
        let store_id = StoreId::new();

        // Create cancelled invoice
        let invoice = InvoiceData {
            id: invoice_id.clone(),
            store_id,
            network: Network::Ethereum,
            status: InvoiceStatus::Cancelled,
            amount: "1000000000000000000".to_string(),
            amount_received: "1000000000000000000".to_string(),
            asset_symbol: "ETH".to_string(),
            payment_address: None,
            payment_request: None,
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            metadata: None,
            extra: None,
        };
        InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

        // Create a payment record
        let tx_hash = B256::repeat_byte(0xab);
        let payment = PaymentData {
            id: Uuid::new_v4(),
            invoice_id: invoice_id.clone(),
            network: Network::Ethereum,
            asset_type: types::AssetType::Native,
            amount: "1000000000000000000".to_string(),
            asset_symbol: "ETH".to_string(),
            token_address: None,
            tx_hash: format!("{:#x}", tx_hash),
            block_number: Some(12345678),
            detected_at: Utc::now(),
            confirmed_at: None,
            from_address: None,
            reorged: false,
            extra: None,
        };
        PaymentWriter::upsert(&*ds, &payment).await.unwrap();

        // Create PaymentConfirmed event
        let event = PaymentConfirmed {
            chain_id: 1,
            invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
            payment_address: Address::ZERO,
            amount: U256::from(1000000000000000000u64),
            tx_hash,
            block_number: 12345678,
            confirmations: 12,
            confirmed_at: Utc::now(),
        };

        // Handle the event
        consumer.handle_payment_confirmed(event).await.unwrap();

        // Invoice should still be cancelled
        let invoice = InvoiceReader::get(&*ds, &invoice_id).await.unwrap().unwrap();
        assert_eq!(invoice.status, InvoiceStatus::Cancelled);
    }

    #[tokio::test]
    async fn test_handle_reorg_detected() {
        let ds = Arc::new(InMemoryDataService::new());
        let bridge = Arc::new(MemoryBridge::new());

        let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> = EventConsumer::new(
            bridge.clone(),
            ds.clone(),
            None,
            None,
        );

        let invoice_id = InvoiceId::new();
        let store_id = StoreId::new();

        // Create invoice in processing state
        let invoice = InvoiceData {
            id: invoice_id.clone(),
            store_id,
            network: Network::Ethereum,
            status: InvoiceStatus::Processing,
            amount: "1000000000000000000".to_string(),
            amount_received: "500000000000000000".to_string(),
            asset_symbol: "ETH".to_string(),
            payment_address: None,
            payment_request: None,
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            metadata: None,
            extra: None,
        };
        InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

        // Create a payment at block 100
        let payment = PaymentData {
            id: Uuid::new_v4(),
            invoice_id: invoice_id.clone(),
            network: Network::Ethereum,
            asset_type: types::AssetType::Native,
            amount: "500000000000000000".to_string(),
            asset_symbol: "ETH".to_string(),
            token_address: None,
            tx_hash: "0xabc123".to_string(),
            block_number: Some(100),
            detected_at: Utc::now(),
            confirmed_at: None,
            from_address: None,
            reorged: false,
            extra: None,
        };
        PaymentWriter::upsert(&*ds, &payment).await.unwrap();

        // Create ReorgDetected event at block 99 (affecting block 100)
        let event = ReorgDetected {
            chain_id: 1,
            fork_block: 99,
            old_hash: B256::ZERO,
            new_hash: B256::repeat_byte(0x01),
            depth: 2,
            affected_invoices: vec![uuid::Uuid::parse_str(invoice_id.as_str()).unwrap()],
            detected_at: Utc::now(),
        };

        // Handle the event
        consumer.handle_reorg_detected(event).await.unwrap();

        // Verify payment was marked as reorged
        let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id).await.unwrap();
        assert!(payments[0].reorged);

        // Verify invoice was reverted to pending (no valid payments)
        let invoice = InvoiceReader::get(&*ds, &invoice_id).await.unwrap().unwrap();
        assert_eq!(invoice.status, InvoiceStatus::Pending);
    }

    #[tokio::test]
    async fn test_handle_reorg_with_remaining_valid_payments() {
        let ds = Arc::new(InMemoryDataService::new());
        let bridge = Arc::new(MemoryBridge::new());

        let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> = EventConsumer::new(
            bridge.clone(),
            ds.clone(),
            None,
            None,
        );

        let invoice_id = InvoiceId::new();
        let store_id = StoreId::new();

        // Create invoice in processing state
        let invoice = InvoiceData {
            id: invoice_id.clone(),
            store_id,
            network: Network::Ethereum,
            status: InvoiceStatus::Processing,
            amount: "1000000000000000000".to_string(),
            amount_received: "1000000000000000000".to_string(),
            asset_symbol: "ETH".to_string(),
            payment_address: None,
            payment_request: None,
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            metadata: None,
            extra: None,
        };
        InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

        // Create first payment at block 50 (will NOT be reorged)
        let payment1 = PaymentData {
            id: Uuid::new_v4(),
            invoice_id: invoice_id.clone(),
            network: Network::Ethereum,
            asset_type: types::AssetType::Native,
            amount: "500000000000000000".to_string(),
            asset_symbol: "ETH".to_string(),
            token_address: None,
            tx_hash: "0xearly".to_string(),
            block_number: Some(50),
            detected_at: Utc::now(),
            confirmed_at: None,
            from_address: None,
            reorged: false,
            extra: None,
        };
        PaymentWriter::upsert(&*ds, &payment1).await.unwrap();

        // Create second payment at block 100 (will be reorged)
        let payment2 = PaymentData {
            id: Uuid::new_v4(),
            invoice_id: invoice_id.clone(),
            network: Network::Ethereum,
            asset_type: types::AssetType::Native,
            amount: "500000000000000000".to_string(),
            asset_symbol: "ETH".to_string(),
            token_address: None,
            tx_hash: "0xlate".to_string(),
            block_number: Some(100),
            detected_at: Utc::now(),
            confirmed_at: None,
            from_address: None,
            reorged: false,
            extra: None,
        };
        PaymentWriter::upsert(&*ds, &payment2).await.unwrap();

        // Create ReorgDetected event at block 99
        let event = ReorgDetected {
            chain_id: 1,
            fork_block: 99,
            old_hash: B256::ZERO,
            new_hash: B256::repeat_byte(0x01),
            depth: 2,
            affected_invoices: vec![uuid::Uuid::parse_str(invoice_id.as_str()).unwrap()],
            detected_at: Utc::now(),
        };

        // Handle the event
        consumer.handle_reorg_detected(event).await.unwrap();

        // Verify only one payment was reorged
        let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id).await.unwrap();
        let reorged_count = payments.iter().filter(|p| p.reorged).count();
        assert_eq!(reorged_count, 1);

        // Invoice should be processing (still has valid payments)
        let invoice = InvoiceReader::get(&*ds, &invoice_id).await.unwrap().unwrap();
        assert_eq!(invoice.status, InvoiceStatus::Processing);
    }

    #[tokio::test]
    async fn test_handle_payment_confirmed_late_payment_on_expired_invoice() {
        let ds = Arc::new(InMemoryDataService::new());
        let bridge = Arc::new(MemoryBridge::new());

        let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> = EventConsumer::new(
            bridge.clone(),
            ds.clone(),
            None,
            None,
        );

        let invoice_id = InvoiceId::new();
        let store_id = StoreId::new();

        // Create an expired invoice (with full amount received - late payment scenario)
        let invoice = InvoiceData {
            id: invoice_id.clone(),
            store_id,
            network: Network::Ethereum,
            status: InvoiceStatus::Expired,
            amount: "1000000000000000000".to_string(), // 1 ETH
            amount_received: "1000000000000000000".to_string(), // Full amount received after expiry
            asset_symbol: "ETH".to_string(),
            payment_address: Some("0x1234567890abcdef1234567890abcdef12345678".to_string()),
            payment_request: None,
            created_at: Utc::now() - chrono::Duration::hours(2),
            expires_at: Utc::now() - chrono::Duration::hours(1), // Expired an hour ago
            metadata: None,
            extra: None,
        };
        InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

        // Create a payment record (detected after expiry)
        let tx_hash = B256::repeat_byte(0xcc);
        let payment = PaymentData {
            id: Uuid::new_v4(),
            invoice_id: invoice_id.clone(),
            network: Network::Ethereum,
            asset_type: types::AssetType::Native,
            amount: "1000000000000000000".to_string(),
            asset_symbol: "ETH".to_string(),
            token_address: None,
            tx_hash: format!("{:#x}", tx_hash),
            block_number: Some(12345700),
            detected_at: Utc::now(),
            confirmed_at: None,
            from_address: Some("0xcccccccccccccccccccccccccccccccccccccccc".to_string()),
            reorged: false,
            extra: None,
        };
        PaymentWriter::upsert(&*ds, &payment).await.unwrap();

        // Create PaymentConfirmed event for the late payment
        let event = PaymentConfirmed {
            chain_id: 1,
            invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
            payment_address: Address::ZERO,
            amount: U256::from(1000000000000000000u64),
            tx_hash,
            block_number: 12345700,
            confirmations: 12,
            confirmed_at: Utc::now(),
        };

        // Handle the event
        consumer.handle_payment_confirmed(event).await.unwrap();

        // Verify invoice was marked as LatePaid (not Paid)
        let invoice = InvoiceReader::get(&*ds, &invoice_id).await.unwrap().unwrap();
        assert_eq!(invoice.status, InvoiceStatus::LatePaid);

        // Verify payment was marked as confirmed
        let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id).await.unwrap();
        assert!(payments[0].confirmed_at.is_some());
    }

    #[test]
    fn test_event_consumer_error_display() {
        let db_err = EventConsumerError::Database(types::RepositoryError::NotFound("test".into()));
        assert!(db_err.to_string().contains("database error"));

        let data_err = EventConsumerError::InvalidData("bad data".into());
        assert!(data_err.to_string().contains("invalid data"));
    }
}
