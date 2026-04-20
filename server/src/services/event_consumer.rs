//! Event consumer service for processing monitor events.
//!
//! Subscribes to evmmonitor events via the EventBridge and updates
//! invoice/payment state in the database.

use std::sync::Arc;

use chrono::Utc;
use data_service::{PaymentOptionReader, StoreWebhookReader};
use evm::monitor::bridge::EventBridge;
use evm::monitor::events::{MonitorEvent, PaymentConfirmed, PaymentDetected, ReorgDetected};
use evm::{chain_id_to_network, get_any_chain_config};
use rust_decimal::Decimal;
use tokio_stream::StreamExt;
use types::{
    AssetType, InvoiceData, InvoiceId, InvoiceReader, InvoiceStatus, InvoiceWriter, PaymentData,
    PaymentReader, PaymentWriter, StoreSettingsReader, TokenReader, WatchedAddressReader,
};
use uuid::Uuid;

use super::evm_monitor::EVMMonitor;
use super::invoice_cleanup::{CleanupDataService, InvoiceCleanupService};
use super::webhook::{
    WebhookDataService, WebhookEventType, WebhookJob, WebhookPayload, WebhookPaymentInfo,
    WebhookService,
};
use crate::api::ws::{StatusUpdate, WsBroadcast};
use crate::metrics;

/// Trait for data service requirements in EventConsumer.
pub trait EventConsumerDataService:
    InvoiceReader
    + InvoiceWriter
    + PaymentReader
    + PaymentWriter
    + PaymentOptionReader
    + TokenReader
    + WatchedAddressReader
    + StoreWebhookReader
    + StoreSettingsReader
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
        + PaymentOptionReader
        + TokenReader
        + WatchedAddressReader
        + StoreWebhookReader
        + StoreSettingsReader
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
    ws_broadcast: Option<Arc<WsBroadcast>>,
}

impl<
    D: EventConsumerDataService + 'static,
    M: EVMMonitor + 'static,
    W: WebhookDataService + 'static,
> EventConsumer<D, M, W>
{
    /// Create a new event consumer with optional services.
    pub fn new(
        bridge: Arc<dyn EventBridge>,
        data_service: Arc<D>,
        cleanup_service: Option<Arc<InvoiceCleanupService<D, M, W>>>,
        webhook_service: Option<Arc<WebhookService<W>>>,
        ws_broadcast: Option<Arc<WsBroadcast>>,
    ) -> Self {
        Self {
            bridge,
            data_service,
            cleanup_service,
            webhook_service,
            ws_broadcast,
        }
    }

    /// Run the event consumer as a background task.
    ///
    /// This should be spawned with `tokio::spawn(consumer.run())`.
    #[allow(clippy::cognitive_complexity)] // event-loop with stream + error handling
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
    #[allow(clippy::cognitive_complexity)] // match on event variants with per-variant logging
    async fn handle_event(&self, event: MonitorEvent) -> Result<(), EventConsumerError> {
        match event {
            MonitorEvent::PaymentDetected(payment) => self.handle_payment_detected(payment).await,
            MonitorEvent::PaymentConfirmed(payment) => self.handle_payment_confirmed(payment).await,
            MonitorEvent::ReorgDetected(reorg) => self.handle_reorg_detected(reorg).await,
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
    #[allow(clippy::too_many_lines, clippy::cognitive_complexity)] // payment state machine — single logical transaction
    async fn handle_payment_detected(
        &self,
        event: PaymentDetected,
    ) -> Result<(), EventConsumerError> {
        // Try to get network from chain_id (None for testnets)
        let network = chain_id_to_network(event.chain_id);

        // Determine asset type and symbol based on whether it's native or token
        let (asset_type, asset_symbol, token_address) = if event.is_native {
            // Get native symbol from chain config (works for both mainnets and testnets)
            let symbol = get_any_chain_config(event.chain_id)
                .map(|c| c.native_symbol.to_string())
                .unwrap_or_else(|| "ETH".to_string());
            (AssetType::Native, symbol, None)
        } else {
            // Look up the token symbol from the database
            let token_addr = event.token_address.ok_or_else(|| {
                EventConsumerError::InvalidData("ERC20 payment missing token_address".to_string())
            })?;
            let token_addr_str = format!("{:#x}", token_addr);

            // Try to look up token in DB (only if we have a known network)
            let symbol = if let Some(net) = network {
                match TokenReader::get_by_address(&*self.data_service, net, &token_addr_str).await?
                {
                    Some(token) => token.symbol.unwrap_or_else(|| "ERC20".to_string()),
                    None => {
                        tracing::warn!(
                            token_address = %token_addr_str,
                            chain_id = event.chain_id,
                            "Unknown token, using address as symbol"
                        );
                        format!("0x{}...", &token_addr_str[2..8])
                    }
                }
            } else {
                // Testnet - use shortened address as symbol
                format!("0x{}...", &token_addr_str[2..8])
            };

            (AssetType::ERC20, symbol, Some(token_addr_str))
        };

        // Look up the payment option by the payment address
        let payment_address_str = format!("{:#x}", event.payment_address);
        let payment_option_id = WatchedAddressReader::get_payment_option_id(
            &*self.data_service,
            &payment_address_str,
            event.chain_id,
            token_address.as_deref(),
        )
        .await?;

        if payment_option_id.is_none() {
            tracing::warn!(
                invoice_id = %event.invoice_id,
                address = %payment_address_str,
                chain_id = event.chain_id,
                amount = %event.amount,
                "Payment detected but no payment option found - payment will be recorded but NOT counted toward invoice total"
            );
        }

        // Calculate converted amount if we have a payment option with rate info
        // IMPORTANT: Payments without credited_amount won't count toward amount_received
        let (credited_amount, rate_used, rate_applied_at) = if let Some(ref po_id) =
            payment_option_id
        {
            match PaymentOptionReader::get(&*self.data_service, po_id).await? {
                Some(payment_option) => {
                    if let Some(ref rate_str) = payment_option.rate {
                        // Convert payment amount to invoice currency
                        // Formula: (raw_amount / 10^decimals) / rate = invoice_currency_amount
                        match self.convert_payment_to_invoice_currency(
                            &event.amount.to_string(),
                            rate_str,
                            payment_option.decimals,
                        ) {
                            Ok(converted) => {
                                tracing::debug!(
                                    invoice_id = %event.invoice_id,
                                    raw_amount = %event.amount,
                                    rate = %rate_str,
                                    decimals = payment_option.decimals,
                                    converted = %converted,
                                    "Converted payment amount to invoice currency"
                                );
                                (Some(converted), Some(rate_str.clone()), Some(Utc::now()))
                            }
                            Err(e) => {
                                tracing::warn!(
                                    invoice_id = %event.invoice_id,
                                    raw_amount = %event.amount,
                                    error = %e,
                                    "Failed to convert payment amount - payment will NOT count toward invoice total"
                                );
                                (None, None, None)
                            }
                        }
                    } else {
                        // No rate = asset-denominated invoice, convert to human-readable
                        match self.convert_smallest_to_human(
                            &event.amount.to_string(),
                            payment_option.decimals,
                        ) {
                            Ok(human_amount) => {
                                tracing::debug!(
                                    invoice_id = %event.invoice_id,
                                    raw_amount = %event.amount,
                                    decimals = payment_option.decimals,
                                    human_amount = %human_amount,
                                    "Same-asset payment, converted to human-readable"
                                );
                                (Some(human_amount), None, None)
                            }
                            Err(e) => {
                                tracing::warn!(
                                    invoice_id = %event.invoice_id,
                                    raw_amount = %event.amount,
                                    error = %e,
                                    "Failed to convert same-asset payment - payment will NOT count toward invoice total"
                                );
                                (None, None, None)
                            }
                        }
                    }
                }
                None => {
                    tracing::warn!(
                        invoice_id = %event.invoice_id,
                        payment_option_id = %po_id.0,
                        "Payment option not found in database - payment will NOT count toward invoice total"
                    );
                    (None, None, None)
                }
            }
        } else {
            (None, None, None)
        };

        // Record metrics before asset_symbol is moved into PaymentData
        metrics::record_payment_detected(event.chain_id, &asset_symbol);

        let payment = PaymentData {
            id: Uuid::new_v4(),
            invoice_id: InvoiceId::from_string(event.invoice_id.to_string()),
            payment_option_id: payment_option_id.map(|id| id.0),
            chain_id: event.chain_id,
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
            credited_amount,
            rate_used,
            rate_applied_at,
        };

        tracing::info!(
            invoice_id = %event.invoice_id,
            tx_hash = %payment.tx_hash,
            amount = %payment.amount,
            chain_id = event.chain_id,
            asset_type = ?asset_type,
            block_number = event.block_number,
            "Payment detected"
        );

        PaymentWriter::upsert(&*self.data_service, &payment).await?;

        // Broadcast payment detected via WebSocket
        if let Some(ref ws) = self.ws_broadcast {
            ws.send(StatusUpdate::PaymentUpdate {
                payment_id: payment.id.to_string(),
                invoice_id: event.invoice_id.to_string(),
                status: "detected".to_string(),
                amount: payment.credited_amount.clone(),
            });
        }

        // Queue webhook notification
        let invoice_id = InvoiceId::from_string(event.invoice_id.to_string());
        if let Ok(Some(invoice)) = InvoiceReader::get(&*self.data_service, &invoice_id).await {
            self.queue_webhook(WebhookEventType::PaymentDetected, &invoice, Some(&payment))
                .await;
        }

        Ok(())
    }

    /// Handle PaymentConfirmed event.
    ///
    /// Updates payment confirmation status and transitions invoice to `paid`
    /// if amount_received >= amount.
    #[allow(clippy::too_many_lines, clippy::cognitive_complexity)] // confirmation state machine — reads/updates/transitions in one flow
    async fn handle_payment_confirmed(
        &self,
        event: PaymentConfirmed,
    ) -> Result<(), EventConsumerError> {
        let invoice_id = InvoiceId::from_string(event.invoice_id.to_string());
        let tx_hash = format!("{:#x}", event.tx_hash);

        // Find the payment by invoice_id + tx_hash (only non-reorged payments)
        let payments =
            PaymentReader::get_valid_for_invoice(&*self.data_service, &invoice_id).await?;
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
        PaymentWriter::mark_confirmed(&*self.data_service, payment.id, event.confirmed_at).await?;

        tracing::info!(
            invoice_id = %event.invoice_id,
            tx_hash = %tx_hash,
            "Payment confirmed"
        );

        // Record metrics
        metrics::record_payment_confirmed(event.chain_id, &payment.asset_symbol);

        // Record confirmation duration (detected_at → confirmed_at)
        if let Ok(duration) = (event.confirmed_at - payment.detected_at).to_std() {
            metrics::record_payment_confirmation_duration(
                event.chain_id,
                &payment.asset_symbol,
                duration,
            );
        }

        // Check if invoice is fully paid
        let invoice = InvoiceReader::get(&*self.data_service, &invoice_id)
            .await?
            .ok_or_else(|| {
                EventConsumerError::InvalidData(format!("Invoice not found: {}", event.invoice_id))
            })?;

        // Compare amounts using rust_decimal
        let amount_received: Decimal = invoice.amount_received.parse().map_err(|e| {
            EventConsumerError::InvalidData(format!(
                "Invalid amount_received '{}': {}",
                invoice.amount_received, e
            ))
        })?;
        let amount_expected: Decimal = invoice.amount.parse().map_err(|e| {
            EventConsumerError::InvalidData(format!("Invalid amount '{}': {}", invoice.amount, e))
        })?;

        let is_fully_paid = amount_received >= amount_expected;

        // Handle based on invoice status
        match invoice.status {
            InvoiceStatus::Processing | InvoiceStatus::PartiallyPaid => {
                // Normal flow: transition to paid if fully paid
                if is_fully_paid {
                    InvoiceWriter::update_status(
                        &*self.data_service,
                        &invoice_id,
                        InvoiceStatus::Paid,
                    )
                    .await?;
                    tracing::info!(
                        invoice_id = %event.invoice_id,
                        amount_received = %amount_received,
                        amount_expected = %amount_expected,
                        "Invoice fully paid"
                    );
                    metrics::record_invoice_paid();

                    // Broadcast invoice paid via WebSocket
                    if let Some(ref ws) = self.ws_broadcast {
                        ws.send(StatusUpdate::InvoiceStatus {
                            invoice_id: event.invoice_id.to_string(),
                            status: InvoiceStatus::Paid.to_string(),
                        });
                    }

                    // Queue webhook notification for payment confirmed
                    if let Ok(Some(updated_invoice)) =
                        InvoiceReader::get(&*self.data_service, &invoice_id).await
                    {
                        self.queue_webhook(
                            WebhookEventType::PaymentConfirmed,
                            &updated_invoice,
                            Some(payment),
                        )
                        .await;
                    }
                }
            }
            InvoiceStatus::Expired => {
                // Late payment: invoice expired but payment still came through
                // Transition to LatePaid for merchant review
                if is_fully_paid {
                    InvoiceWriter::update_status(
                        &*self.data_service,
                        &invoice_id,
                        InvoiceStatus::LatePaid,
                    )
                    .await?;
                    tracing::warn!(
                        invoice_id = %event.invoice_id,
                        amount_received = %amount_received,
                        amount_expected = %amount_expected,
                        "Late payment received on expired invoice - requires merchant review"
                    );

                    // Broadcast late payment via WebSocket
                    if let Some(ref ws) = self.ws_broadcast {
                        ws.send(StatusUpdate::InvoiceStatus {
                            invoice_id: event.invoice_id.to_string(),
                            status: InvoiceStatus::LatePaid.to_string(),
                        });
                    }

                    // Queue webhook notification for late payment
                    if let Ok(Some(updated_invoice)) =
                        InvoiceReader::get(&*self.data_service, &invoice_id).await
                    {
                        self.queue_webhook(
                            WebhookEventType::LatePaid,
                            &updated_invoice,
                            Some(payment),
                        )
                        .await;
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

    /// Trigger invoice expiration check.
    ///
    /// Called when block events are received. This is a non-blocking operation -
    /// errors are logged but don't stop event processing.
    ///
    /// With network-agnostic invoices, this checks all expired invoices regardless
    /// of which chain triggered the event.
    async fn trigger_expiration_check(&self, chain_id: u64) {
        let Some(cleanup_service) = &self.cleanup_service else {
            return;
        };

        match cleanup_service.check_chain(chain_id).await {
            Ok(count) => {
                if count > 0 {
                    tracing::debug!(
                        chain_id,
                        expired_count = count,
                        "Expired invoices on block event"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    chain_id,
                    error = %e,
                    "Failed to check expired invoices"
                );
            }
        }
    }

    /// Queue a webhook notification for an invoice status change.
    ///
    /// This is a non-blocking operation - errors are logged but don't stop event processing.
    #[allow(clippy::cognitive_complexity)] // webhook payload assembly with optional fields
    async fn queue_webhook(
        &self,
        event_type: WebhookEventType,
        invoice: &InvoiceData,
        payment: Option<&PaymentData>,
    ) {
        let Some(webhook_service) = &self.webhook_service else {
            return;
        };

        // Check notification preferences — skip if webhook disabled for this event
        if let Ok(Some(settings)) =
            StoreSettingsReader::get_store_settings(&*self.data_service, invoice.store_id.0).await
        {
            let event_key = event_type.to_string();
            if let Some(event_prefs) = settings.notification_prefs.get(&event_key)
                && event_prefs.get("webhook") == Some(&serde_json::Value::Bool(false))
            {
                tracing::trace!(
                    store_id = %invoice.store_id.0,
                    event = %event_key,
                    "Webhook suppressed by notification_prefs"
                );
                return;
            }
        }

        // Look up webhook config for the store
        let webhook_config =
            match StoreWebhookReader::get_enabled_webhook(&*self.data_service, invoice.store_id.0)
                .await
            {
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
        // With network-agnostic invoices, chain info comes from the payment
        let (asset_symbol, chain_id, network) = if let Some(p) = payment {
            let network = chain_id_to_network(p.chain_id);
            (
                p.asset_symbol.clone(),
                Some(p.chain_id),
                network.map(|n| n.to_string()),
            )
        } else {
            // No payment yet, use invoice currency as a placeholder
            (invoice.currency.clone(), None, None)
        };

        let payload = WebhookPayload {
            event_id: Uuid::new_v4(),
            event_type,
            timestamp: chrono::Utc::now(),
            invoice_id: invoice.id.as_str().to_string(),
            store_id: invoice.store_id.0,
            status: invoice.status.to_string(),
            amount: invoice.amount.clone(),
            amount_received: invoice.amount_received.clone(),
            asset_symbol,
            chain_id: chain_id.unwrap_or(0),
            network,
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
    #[allow(clippy::cognitive_complexity)] // reorg recovery with multi-payment rollback logic
    async fn handle_reorg_detected(&self, event: ReorgDetected) -> Result<(), EventConsumerError> {
        // Try to get network from chain_id (None for testnets)
        let network = chain_id_to_network(event.chain_id);

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
            let reorged_count = self
                .data_service
                .mark_reorged(&invoice_id, event.chain_id, event.fork_block)
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
            let has_valid =
                PaymentReader::has_valid_payments(&*self.data_service, &invoice_id).await?;
            let new_status = if has_valid {
                InvoiceStatus::Processing
            } else {
                InvoiceStatus::Pending
            };

            InvoiceWriter::update_status(&*self.data_service, &invoice_id, new_status).await?;

            // Broadcast reorg-induced status change via WebSocket
            if let Some(ref ws) = self.ws_broadcast {
                ws.send(StatusUpdate::InvoiceStatus {
                    invoice_id: invoice_uuid.to_string(),
                    status: new_status.to_string(),
                });
            }

            tracing::info!(
                invoice_id = %invoice_uuid,
                new_status = ?new_status,
                "Reverted invoice status after reorg"
            );
        }

        Ok(())
    }

    /// Convert a payment amount (in smallest units) to invoice currency.
    ///
    /// Formula: (raw_amount / 10^decimals) / rate = invoice_currency_amount
    ///
    /// The rate represents: 1 invoice_currency = rate asset_units
    /// So to get invoice currency: asset_amount / rate
    fn convert_payment_to_invoice_currency(
        &self,
        raw_amount: &str,
        rate_str: &str,
        decimals: u8,
    ) -> Result<String, String> {
        // Parse raw amount (in smallest units, e.g., wei)
        let raw: Decimal = raw_amount
            .parse()
            .map_err(|e| format!("Invalid raw amount '{}': {}", raw_amount, e))?;

        // Parse exchange rate
        let rate: Decimal = rate_str
            .parse()
            .map_err(|e| format!("Invalid rate '{}': {}", rate_str, e))?;

        if rate.is_zero() {
            return Err("Rate is zero, cannot convert".to_string());
        }

        // Convert to human-readable amount: raw / 10^decimals
        let divisor = Self::compute_decimal_divisor(decimals)?;
        let human_amount = raw / divisor;

        // Convert to invoice currency: human_amount / rate
        let invoice_amount = human_amount / rate;

        Ok(invoice_amount.to_string())
    }

    /// Convert a smallest unit amount to human-readable format.
    ///
    /// Used for asset-denominated invoices where no rate conversion is needed.
    fn convert_smallest_to_human(&self, raw_amount: &str, decimals: u8) -> Result<String, String> {
        let raw: Decimal = raw_amount
            .parse()
            .map_err(|e| format!("Invalid raw amount '{}': {}", raw_amount, e))?;

        let divisor = Self::compute_decimal_divisor(decimals)?;
        let human_amount = raw / divisor;

        Ok(human_amount.to_string())
    }

    /// Compute 10^decimals safely using checked multiplication.
    fn compute_decimal_divisor(decimals: u8) -> Result<Decimal, String> {
        let ten = Decimal::from(10);
        let mut divisor = Decimal::ONE;
        for _ in 0..decimals {
            divisor = divisor
                .checked_mul(ten)
                .ok_or_else(|| format!("Overflow computing 10^{}", decimals))?;
        }
        Ok(divisor)
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use async_trait::async_trait;

    /// Get the native asset symbol for a network (test helper).
    fn network_native_symbol(network: types::Network) -> String {
        use types::Network::*;
        match network {
            Ethereum | Arbitrum | Optimism | Base | ZkSync | Linea | Scroll => "ETH",
            Polygon => "POL",
            Avalanche => "AVAX",
            BinanceSmartChain => "BNB",
            Fantom => "FTM",
            Gnosis => "xDAI",
            // Non-EVM networks - shouldn't reach here
            _ => "UNKNOWN",
        }
        .to_string()
    }
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

        async fn watch_address_by_chain_id(
            &self,
            _chain_id: u64,
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
            _token_contract: Option<Address>,
        ) -> Result<(), EVMMonitorError> {
            Ok(())
        }

        async fn unwatch_address_by_chain_id(
            &self,
            _chain_id: u64,
            _address: Address,
            _token_contract: Option<Address>,
        ) -> Result<(), EVMMonitorError> {
            Ok(())
        }

        async fn health_check(&self) -> Result<(), EVMMonitorError> {
            Ok(())
        }

        async fn get_chain_health(
            &self,
        ) -> Result<Vec<evm::monitor::ChainHealth>, EVMMonitorError> {
            Ok(vec![])
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
            currency: "ETH".to_string(),
            status: InvoiceStatus::Pending,
            amount: "1000000000000000000".to_string(), // 1 ETH
            amount_received: "0".to_string(),
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
        assert_eq!(network_native_symbol(Network::Fantom), "FTM");
        assert_eq!(network_native_symbol(Network::Gnosis), "xDAI");
        // Non-EVM networks
        assert_eq!(network_native_symbol(Network::BitcoinMainnet), "UNKNOWN");
    }

    #[tokio::test]
    async fn test_handle_payment_detected_native() {
        let ds = Arc::new(InMemoryDataService::new());
        let bridge = Arc::new(MemoryBridge::new());

        let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> =
            EventConsumer::new(bridge.clone(), ds.clone(), None, None, None);

        let invoice_id = InvoiceId::new();
        let store_id = StoreId::new();
        create_test_invoice(&ds, &invoice_id, store_id).await;

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
        let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
            .await
            .unwrap();
        assert_eq!(payments.len(), 1);
        assert_eq!(payments[0].asset_symbol, "ETH");
        assert_eq!(payments[0].amount, "500000000000000000");
        assert!(!payments[0].reorged);
    }

    #[tokio::test]
    async fn test_handle_payment_detected_unknown_chain() {
        // With network-agnostic approach, payments from unknown chains are accepted
        // chain_id is stored on the payment, not the invoice
        let ds = Arc::new(InMemoryDataService::new());
        let bridge = Arc::new(MemoryBridge::new());

        let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> =
            EventConsumer::new(bridge.clone(), ds.clone(), None, None, None);

        let invoice_id = InvoiceId::new();
        let store_id = StoreId::new();

        // Create network-agnostic invoice
        let invoice = InvoiceData {
            id: invoice_id.clone(),
            store_id,
            currency: "ETH".to_string(),
            status: InvoiceStatus::Pending,
            amount: "1000000000000000000".to_string(),
            amount_received: "0".to_string(),
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            metadata: None,
            extra: None,
        };
        InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

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

        // Should succeed - unknown chains are now accepted
        consumer.handle_payment_detected(event).await.unwrap();

        // Verify payment was created with chain_id
        let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
            .await
            .unwrap();
        assert_eq!(payments.len(), 1);
        assert_eq!(payments[0].chain_id, 99999);
        assert_eq!(payments[0].asset_symbol, "ETH"); // Fallback for unknown chains
    }

    #[tokio::test]
    async fn test_handle_payment_confirmed_transitions_to_paid() {
        let ds = Arc::new(InMemoryDataService::new());
        let bridge = Arc::new(MemoryBridge::new());

        let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> =
            EventConsumer::new(bridge.clone(), ds.clone(), None, None, None);

        let invoice_id = InvoiceId::new();
        let store_id = StoreId::new();

        // Create invoice in processing state with full amount received
        let invoice = InvoiceData {
            id: invoice_id.clone(),
            store_id,
            currency: "ETH".to_string(),
            status: InvoiceStatus::Processing,
            amount: "1000000000000000000".to_string(), // 1 ETH
            amount_received: "1000000000000000000".to_string(), // 1 ETH received
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
            payment_option_id: None,
            chain_id: 1,
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
            credited_amount: Some("1".to_string()), // 1 ETH
            rate_used: None,
            rate_applied_at: None,
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
        let invoice = InvoiceReader::get(&*ds, &invoice_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(invoice.status, InvoiceStatus::Paid);

        // Verify payment was marked as confirmed
        let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
            .await
            .unwrap();
        assert!(payments[0].confirmed_at.is_some());
    }

    #[tokio::test]
    async fn test_handle_payment_confirmed_skips_cancelled_invoice() {
        let ds = Arc::new(InMemoryDataService::new());
        let bridge = Arc::new(MemoryBridge::new());

        let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> =
            EventConsumer::new(bridge.clone(), ds.clone(), None, None, None);

        let invoice_id = InvoiceId::new();
        let store_id = StoreId::new();

        // Create cancelled invoice
        let invoice = InvoiceData {
            id: invoice_id.clone(),
            store_id,
            currency: "ETH".to_string(),
            status: InvoiceStatus::Cancelled,
            amount: "1000000000000000000".to_string(),
            amount_received: "1000000000000000000".to_string(),
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
            payment_option_id: None,
            chain_id: 1,
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
            credited_amount: Some("1".to_string()),
            rate_used: None,
            rate_applied_at: None,
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
        let invoice = InvoiceReader::get(&*ds, &invoice_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(invoice.status, InvoiceStatus::Cancelled);
    }

    #[tokio::test]
    async fn test_handle_reorg_detected() {
        let ds = Arc::new(InMemoryDataService::new());
        let bridge = Arc::new(MemoryBridge::new());

        let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> =
            EventConsumer::new(bridge.clone(), ds.clone(), None, None, None);

        let invoice_id = InvoiceId::new();
        let store_id = StoreId::new();

        // Create invoice in processing state
        let invoice = InvoiceData {
            id: invoice_id.clone(),
            store_id,
            currency: "ETH".to_string(),
            status: InvoiceStatus::Processing,
            amount: "1000000000000000000".to_string(),
            amount_received: "500000000000000000".to_string(),
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
            payment_option_id: None,
            chain_id: 1,
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
            credited_amount: Some("0.5".to_string()),
            rate_used: None,
            rate_applied_at: None,
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
        let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
            .await
            .unwrap();
        assert!(payments[0].reorged);

        // Verify invoice was reverted to pending (no valid payments)
        let invoice = InvoiceReader::get(&*ds, &invoice_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(invoice.status, InvoiceStatus::Pending);
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // multi-step reorg test with setup + multiple assertions
    async fn test_handle_reorg_with_remaining_valid_payments() {
        let ds = Arc::new(InMemoryDataService::new());
        let bridge = Arc::new(MemoryBridge::new());

        let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> =
            EventConsumer::new(bridge.clone(), ds.clone(), None, None, None);

        let invoice_id = InvoiceId::new();
        let store_id = StoreId::new();

        // Create invoice in processing state
        let invoice = InvoiceData {
            id: invoice_id.clone(),
            store_id,
            currency: "ETH".to_string(),
            status: InvoiceStatus::Processing,
            amount: "1000000000000000000".to_string(),
            amount_received: "1000000000000000000".to_string(),
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
            payment_option_id: None,
            chain_id: 1,
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
            credited_amount: Some("0.5".to_string()),
            rate_used: None,
            rate_applied_at: None,
        };
        PaymentWriter::upsert(&*ds, &payment1).await.unwrap();

        // Create second payment at block 100 (will be reorged)
        let payment2 = PaymentData {
            id: Uuid::new_v4(),
            invoice_id: invoice_id.clone(),
            payment_option_id: None,
            chain_id: 1,
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
            credited_amount: Some("0.5".to_string()),
            rate_used: None,
            rate_applied_at: None,
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
        let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
            .await
            .unwrap();
        let reorged_count = payments.iter().filter(|p| p.reorged).count();
        assert_eq!(reorged_count, 1);

        // Invoice should be processing (still has valid payments)
        let invoice = InvoiceReader::get(&*ds, &invoice_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(invoice.status, InvoiceStatus::Processing);
    }

    #[tokio::test]
    async fn test_handle_payment_confirmed_late_payment_on_expired_invoice() {
        let ds = Arc::new(InMemoryDataService::new());
        let bridge = Arc::new(MemoryBridge::new());

        let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> =
            EventConsumer::new(bridge.clone(), ds.clone(), None, None, None);

        let invoice_id = InvoiceId::new();
        let store_id = StoreId::new();

        // Create an expired invoice (with full amount received - late payment scenario)
        let invoice = InvoiceData {
            id: invoice_id.clone(),
            store_id,
            currency: "ETH".to_string(),
            status: InvoiceStatus::Expired,
            amount: "1000000000000000000".to_string(), // 1 ETH
            amount_received: "1000000000000000000".to_string(), // Full amount received after expiry
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
            payment_option_id: None,
            chain_id: 1,
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
            credited_amount: Some("1".to_string()),
            rate_used: None,
            rate_applied_at: None,
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
        let invoice = InvoiceReader::get(&*ds, &invoice_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(invoice.status, InvoiceStatus::LatePaid);

        // Verify payment was marked as confirmed
        let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
            .await
            .unwrap();
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
