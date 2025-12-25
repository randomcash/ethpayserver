//! Event consumer service for processing monitor events.
//!
//! Subscribes to evmmonitor events via the EventBridge and updates
//! invoice/payment state in the database.

use std::sync::Arc;

use data_service::{PgDataService, StoreWebhookReader};
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

use super::webhook::{
    WebhookEventType, WebhookJob, WebhookPayload, WebhookPaymentInfo, WebhookService,
};
use super::InvoiceExpirationService;

/// Event consumer that processes monitor events and updates database state.
///
/// Optionally sends webhook notifications when invoice status changes.
pub struct EventConsumer {
    bridge: Arc<dyn EventBridge>,
    data_service: Arc<PgDataService>,
    expiration_service: Option<Arc<InvoiceExpirationService>>,
    webhook_service: Option<Arc<WebhookService>>,
}

impl EventConsumer {
    /// Create a new event consumer with optional services.
    pub fn new(
        bridge: Arc<dyn EventBridge>,
        data_service: Arc<PgDataService>,
        expiration_service: Option<Arc<InvoiceExpirationService>>,
        webhook_service: Option<Arc<WebhookService>>,
    ) -> Self {
        Self {
            bridge,
            data_service,
            expiration_service,
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
            confirmations: event.confirmations as u32,
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
            confirmations = event.confirmations,
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

        // Update payment confirmations and confirmed_at
        PaymentWriter::update_confirmations(
            &*self.data_service,
            payment.id,
            event.confirmations as u32,
            Some(event.confirmed_at),
        ).await?;

        tracing::info!(
            invoice_id = %event.invoice_id,
            tx_hash = %tx_hash,
            confirmations = event.confirmations,
            "Payment confirmed"
        );

        // Check if invoice is fully paid
        let invoice = InvoiceReader::get(&*self.data_service, &invoice_id)
            .await?
            .ok_or_else(|| EventConsumerError::InvalidData(
                format!("Invoice not found: {}", event.invoice_id)
            ))?;

        // Only transition to paid if invoice is in a valid state
        // Don't mark cancelled/expired/refunded invoices as paid
        if !matches!(invoice.status, InvoiceStatus::Processing | InvoiceStatus::PartiallyPaid) {
            tracing::debug!(
                invoice_id = %event.invoice_id,
                status = ?invoice.status,
                "Invoice not in payable state, skipping status update"
            );
            return Ok(());
        }

        // Compare amounts using rust_decimal
        let amount_received: Decimal = invoice.amount_received.parse()
            .map_err(|e| EventConsumerError::InvalidData(
                format!("Invalid amount_received '{}': {}", invoice.amount_received, e)
            ))?;
        let amount_expected: Decimal = invoice.amount.parse()
            .map_err(|e| EventConsumerError::InvalidData(
                format!("Invalid amount '{}': {}", invoice.amount, e)
            ))?;

        if amount_received >= amount_expected {
            InvoiceWriter::update_status(&*self.data_service, &invoice_id, InvoiceStatus::Paid).await?;
            tracing::info!(
                invoice_id = %event.invoice_id,
                amount_received = %amount_received,
                amount_expected = %amount_expected,
                "Invoice fully paid"
            );

            // Queue webhook notification for payment confirmed
            // Re-fetch invoice to get updated status
            if let Ok(Some(updated_invoice)) = InvoiceReader::get(&*self.data_service, &invoice_id).await {
                self.queue_webhook(WebhookEventType::PaymentConfirmed, &updated_invoice, Some(payment)).await;
            }
        }

        Ok(())
    }

    /// Trigger invoice expiration check for a network.
    ///
    /// Called when block events are received. This is a non-blocking operation -
    /// errors are logged but don't stop event processing.
    async fn trigger_expiration_check(&self, chain_id: u64) {
        let Some(expiration_service) = &self.expiration_service else {
            return;
        };

        let Some(network) = chain_id_to_network(chain_id) else {
            tracing::trace!(chain_id, "Unknown chain_id for expiration check");
            return;
        };

        match expiration_service.check_network(network).await {
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
                confirmations: p.confirmations,
                from_address: p.from_address.clone(),
                block_number: p.block_number,
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
