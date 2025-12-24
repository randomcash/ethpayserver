//! Event consumer service for processing monitor events.
//!
//! Subscribes to evmmonitor events via the EventBridge and updates
//! invoice/payment state in the database.

use std::sync::Arc;

use data_service::PgDataService;
use evm::chain_id_to_network;
use evm::monitor::bridge::EventBridge;
use evm::monitor::events::{MonitorEvent, PaymentConfirmed, PaymentDetected, ReorgDetected};
use rust_decimal::Decimal;
use tokio_stream::StreamExt;
use types::{InvoiceId, InvoiceReader, InvoiceStatus, InvoiceWriter, PaymentData, PaymentReader, PaymentWriter};
use uuid::Uuid;

/// Event consumer that processes monitor events and updates database state.
///
/// Future extensions:
/// - Webhook notifications on status changes
/// - WebSocket push to connected clients
pub struct EventConsumer {
    bridge: Arc<dyn EventBridge>,
    data_service: Arc<PgDataService>,
}

impl EventConsumer {
    /// Create a new event consumer.
    pub fn new(bridge: Arc<dyn EventBridge>, data_service: Arc<PgDataService>) -> Self {
        Self { bridge, data_service }
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

        // Determine asset symbol based on whether it's native or token
        let asset_symbol = if event.is_native {
            network_native_symbol(network)
        } else {
            // For tokens, we'd need to look up the symbol from the token address
            // For now, use a placeholder - this should be enhanced
            "ERC20".to_string()
        };

        let payment = PaymentData {
            id: Uuid::new_v4(),
            invoice_id: InvoiceId::from_string(event.invoice_id.to_string()),
            network,
            amount: event.amount.to_string(),
            asset_symbol,
            tx_hash: format!("{:#x}", event.tx_hash),
            block_number: Some(event.block_number),
            confirmations: event.confirmations as u32,
            detected_at: event.detected_at,
            confirmed_at: None,
            from_address: Some(format!("{:#x}", event.from_address)),
            extra: None,
        };

        tracing::info!(
            invoice_id = %event.invoice_id,
            tx_hash = %payment.tx_hash,
            amount = %payment.amount,
            network = ?network,
            confirmations = event.confirmations,
            "Payment detected"
        );

        PaymentWriter::upsert(&*self.data_service, &payment).await?;

        Ok(())
    }

    /// Handle PaymentConfirmed event.
    ///
    /// Updates payment confirmation status and transitions invoice to `paid`
    /// if amount_received >= amount.
    async fn handle_payment_confirmed(&self, event: PaymentConfirmed) -> Result<(), EventConsumerError> {
        let invoice_id = InvoiceId::from_string(event.invoice_id.to_string());
        let tx_hash = format!("{:#x}", event.tx_hash);

        // Find the payment by invoice_id + tx_hash
        let payments = PaymentReader::get_for_invoice(&*self.data_service, &invoice_id).await?;
        let payment = payments
            .iter()
            .find(|p| p.tx_hash == tx_hash)
            .ok_or_else(|| EventConsumerError::InvalidData(
                format!("Payment not found: invoice={}, tx={}", event.invoice_id, tx_hash)
            ))?;

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
        }

        Ok(())
    }

    /// Handle ReorgDetected event.
    ///
    /// Marks affected payments as reorged and reverts invoice status:
    /// - If other valid payments exist → `processing`
    /// - If no valid payments → `pending`
    async fn handle_reorg_detected(&self, event: ReorgDetected) -> Result<(), EventConsumerError> {
        tracing::warn!(
            chain_id = event.chain_id,
            fork_block = event.fork_block,
            depth = event.depth,
            affected_invoices = event.affected_invoices.len(),
            "Chain reorganization detected"
        );

        for invoice_uuid in &event.affected_invoices {
            let invoice_id = InvoiceId::from_string(invoice_uuid.to_string());

            // Mark all payments for this invoice as reorged
            let reorged_count = self.data_service.mark_payments_reorged(&invoice_id).await?;

            if reorged_count == 0 {
                tracing::debug!(invoice_id = %invoice_uuid, "No payments to mark as reorged");
                continue;
            }

            tracing::info!(
                invoice_id = %invoice_uuid,
                reorged_count,
                "Marked payments as reorged"
            );

            // Determine new invoice status based on remaining valid payments
            let has_valid = self.data_service.has_valid_payments(&invoice_id).await?;
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
