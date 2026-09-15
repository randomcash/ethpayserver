//! Event consumer service for processing monitor events.
//!
//! Subscribes to evmmonitor events via the EventBridge and updates
//! invoice/payment state in the database.

mod confirmation_handler;
mod payment_handler;
mod reorg_handler;
mod webhook_dispatch;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use auth::StoreRepository;
use bigdecimal::{BigDecimal, RoundingMode, Zero};
use data_service::{PaymentOptionReader, PaymentTxIndexReader, PaymentTxIndexWriter};
use evm::monitor::bridge::EventBridge;
use evm::monitor::events::MonitorEvent;
use tokio_stream::StreamExt;
use types::{
    InvoiceReader, InvoiceWriter, PaymentReader, PaymentWriter, StoreSettingsReader, TokenReader,
    WatchedAddressReader,
};

use super::email::EmailSender;
use super::evm_monitor::EVMMonitor;
use super::invoice_cleanup::{CleanupDataService, InvoiceCleanupService};
use super::webhook::{WebhookDataService, WebhookSink};
use crate::api::ws::WsBroadcast;

/// Trait for data service requirements in EventConsumer.
pub trait EventConsumerDataService:
    InvoiceReader
    + InvoiceWriter
    + PaymentReader
    + PaymentWriter
    + PaymentTxIndexWriter
    + PaymentTxIndexReader
    + PaymentOptionReader
    + TokenReader
    + WatchedAddressReader
    + StoreSettingsReader
    + StoreRepository
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
        + PaymentTxIndexWriter
        + PaymentTxIndexReader
        + PaymentOptionReader
        + TokenReader
        + WatchedAddressReader
        + StoreSettingsReader
        + StoreRepository
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
    webhook_service: Option<Arc<dyn WebhookSink>>,
    ws_broadcast: Option<Arc<WsBroadcast>>,
    email_sender: Arc<dyn EmailSender>,
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
        webhook_service: Option<Arc<dyn WebhookSink>>,
        ws_broadcast: Option<Arc<WsBroadcast>>,
        email_sender: Arc<dyn EmailSender>,
    ) -> Self {
        Self {
            bridge,
            data_service,
            cleanup_service,
            webhook_service,
            ws_broadcast,
            email_sender,
        }
    }

    /// Run the event consumer as a background task.
    ///
    /// This should be spawned with `tokio::spawn(consumer.run())`.
    #[allow(clippy::cognitive_complexity)]
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
    #[allow(clippy::cognitive_complexity)]
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

    /// Convert a payment amount (in smallest units) to invoice currency.
    ///
    /// Formula: (raw_amount / 10^decimals) / rate = invoice_currency_amount
    ///
    /// The rate represents: 1 invoice_currency = rate asset_units
    /// So to get invoice currency: asset_amount / rate
    ///
    /// A raw amount is up to 78 digits (`NUMERIC(78,0)`), well past what
    /// `rust_decimal::Decimal`'s 96-bit mantissa (~28-29 digits) can hold
    /// exactly. `BigDecimal` is arbitrary-precision, so parsing and the
    /// power-of-ten division never round; only the division by `rate` (a
    /// genuinely fractional value) can produce a non-terminating result, and
    /// that's inherent to rate conversion, not a precision bug.
    fn convert_payment_to_invoice_currency(
        &self,
        raw_amount: &str,
        rate_str: &str,
        decimals: u8,
    ) -> Result<String, String> {
        // Parse raw amount (in smallest units, e.g., wei)
        let raw: BigDecimal = raw_amount
            .parse()
            .map_err(|e| format!("Invalid raw amount '{}': {}", raw_amount, e))?;

        // Parse exchange rate
        let rate: BigDecimal = rate_str
            .parse()
            .map_err(|e| format!("Invalid rate '{}': {}", rate_str, e))?;

        if rate.is_zero() {
            return Err("Rate is zero, cannot convert".to_string());
        }

        // Convert to human-readable amount: raw / 10^decimals
        let divisor = Self::compute_decimal_divisor(decimals);
        let human_amount = raw / divisor;

        // Convert to invoice currency: human_amount / rate
        let invoice_amount = human_amount / rate;

        Ok(Self::format_amount(&invoice_amount))
    }

    /// Convert a smallest unit amount to human-readable format.
    ///
    /// Used for asset-denominated invoices where no rate conversion is needed.
    /// This is an exact power-of-ten division (moving the decimal point), so
    /// `BigDecimal` never rounds here regardless of how large `raw_amount` is.
    fn convert_smallest_to_human(&self, raw_amount: &str, decimals: u8) -> Result<String, String> {
        let raw: BigDecimal = raw_amount
            .parse()
            .map_err(|e| format!("Invalid raw amount '{}': {}", raw_amount, e))?;

        let divisor = Self::compute_decimal_divisor(decimals);
        let human_amount = raw / divisor;

        Ok(Self::format_amount(&human_amount))
    }

    /// Compute 10^decimals. `BigDecimal` is arbitrary-precision, so this is
    /// always exact and cannot overflow the way a fixed-mantissa type would.
    fn compute_decimal_divisor(decimals: u8) -> BigDecimal {
        BigDecimal::from(10u8).powi(i64::from(decimals))
    }

    /// Render an amount the way the rest of the system already reads it.
    ///
    /// Two things `BigDecimal::to_string` does that `rust_decimal` did not,
    /// and that reach a user:
    ///
    /// - It switches to scientific notation past five leading zeros, so one
    ///   wei of an 18-decimal token stringifies as `1E-18`. That value is
    ///   broadcast over the checkout and dashboard WebSocket, where a plain
    ///   decimal was shown before.
    /// - Division runs at 100 significant digits rather than 28, so a rate
    ///   conversion can produce a 100-plus character string. It is stored in
    ///   `NUMERIC(78,18)`, so everything past the 18th decimal is dropped on
    ///   write - rounding here means the number sent over the WebSocket and
    ///   the number the invoice API returns later are the same number.
    ///
    /// `normalized` strips the trailing zeros the fixed scale introduces, so
    /// a whole amount stays `1` rather than `1.000000000000000000`.
    fn format_amount(value: &BigDecimal) -> String {
        value
            .with_scale_round(18, RoundingMode::HalfUp)
            .normalized()
            .to_plain_string()
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
