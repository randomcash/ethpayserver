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

use std::collections::HashMap;
use std::sync::Arc;

use auth::StoreRepository;
use bigdecimal::{BigDecimal, RoundingMode, Zero};
use data_service::{
    ChainCursor, ChainCursorReader, ChainCursorWriter, PaymentOptionReader, PaymentTxIndexReader,
    PaymentTxIndexWriter, ReorgCandidateReader, ReorgWriter,
};
use evm::monitor::bridge::{EventBridge, EventCursor, EventEnvelope};
use evm::monitor::events::MonitorEvent;
use evm::EvmError;
use tokio_stream::StreamExt;
use types::{
    InvoiceReader, InvoiceWriter, PaymentReader, PaymentWriter, StoreSettingsReader, TokenReader,
    WatchedAddressReader,
};

use super::email::EmailSender;
use super::evm_monitor::EVMMonitor;
use super::invoice_cleanup::{CleanupDataService, InvoiceCleanupService};
use super::plugins::OwnStorePaymentObserver;
use super::webhook::{WebhookDataService, WebhookSink};
use crate::api::ws::WsBroadcast;

/// Identifies this server's event source when persisting resume cursors.
///
/// A plain constant because there is exactly one adapter kind today
/// (evmmonitor); the column exists so a second adapter would not collide
/// with it, not because this server picks between several.
const ADAPTER_ID: &str = "evmmonitor";

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
    + ReorgCandidateReader
    + ReorgWriter
    + ChainCursorReader
    + ChainCursorWriter
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
        + ReorgCandidateReader
        + ReorgWriter
        + ChainCursorReader
        + ChainCursorWriter
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
    /// Capability 4 observers, and the one store they may hear about.
    ///
    /// Both default to "nothing": an instance that does not sell
    /// subscriptions to itself has no own store and no observers, and
    /// `own_store_id: None` reports nothing even if an observer is somehow
    /// registered. See `plugins::payment_observer`.
    payment_observers: Vec<Arc<dyn OwnStorePaymentObserver>>,
    own_store_id: Option<types::StoreId>,
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
            payment_observers: Vec::new(),
            own_store_id: None,
        }
    }

    /// Report settled invoices on `own_store_id` to `observers`.
    ///
    /// Separate from `new` rather than two more positional arguments: every
    /// existing caller wants neither, and a call site that silently passed
    /// the wrong store here would leak merchants' payments to a plugin. An
    /// instance that never calls this reports nothing, which is the state
    /// every deployment is in until a billing plugin is configured.
    #[must_use]
    pub fn with_own_store_payments(
        mut self,
        own_store_id: types::StoreId,
        observers: Vec<Arc<dyn OwnStorePaymentObserver>>,
    ) -> Self {
        self.own_store_id = Some(own_store_id);
        self.payment_observers = observers;
        self
    }

    /// Run the event consumer as a background task.
    ///
    /// This should be spawned with `tokio::spawn(consumer.run())`.
    #[allow(clippy::cognitive_complexity)]
    pub async fn run(self) {
        tracing::info!("Starting event consumer");

        let mut cursors =
            match ChainCursorReader::chain_cursors(&*self.data_service, ADAPTER_ID).await {
                Ok(c) => c,
                Err(e) => {
                    // A failed load is indistinguishable from an empty
                    // `HashMap` to everything downstream, but they are not
                    // the same thing: an empty map means "never resumed
                    // before, nothing to lose," which skips the
                    // epoch-mismatch check below and resumes from whatever
                    // the outbox currently retains. Silently doing that on a
                    // DB hiccup would drop real cursors this server had.
                    // Refusing to start is the safe failure here.
                    tracing::error!(error = %e, "failed to load chain cursors; refusing to start");
                    return;
                }
            };

        let bridge_epoch = match self.bridge.current_epoch().await {
            Ok(e) => e,
            Err(e) => {
                tracing::error!(error = %e, "failed to read the event outbox's epoch");
                return;
            }
        };

        let mut resume_from = self.reconcile_cursors(&mut cursors, bridge_epoch).await;

        // Bounded to one retry: `subscribe_from` only ever reports
        // out-of-range for a `Some(cursor)` resume target, and the retry
        // below always resumes with `None` - a second out-of-range report
        // after that would mean the bridge itself is broken, not something
        // re-arming watch_retry again can fix.
        let mut retried = false;
        let mut event_stream = loop {
            match self.bridge.subscribe_from(resume_from).await {
                Ok(stream) => break stream,
                Err(EvmError::EventStreamOutOfRange(reason)) if !retried => {
                    tracing::error!(
                        reason = %reason,
                        "resume position no longer retained; re-arming watch_retry and \
                         resuming from the outbox's new oldest entry"
                    );
                    self.break_lineage(&mut cursors).await;
                    resume_from = None;
                    retried = true;
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to subscribe to events");
                    return;
                }
            }
        };

        while let Some(envelope) = event_stream.next().await {
            self.apply_envelope(envelope, &mut cursors).await;
        }

        tracing::warn!("Event stream ended, consumer shutting down");
    }

    /// Reconcile this server's stored cursors against the outbox's current
    /// epoch, returning where to resume.
    ///
    /// A cursor whose epoch still matches is trustworthy: `seq` names a
    /// position in the outbox that still exists, so resuming from the
    /// lowest `seq` across all watched chains - the low-water mark - is
    /// enough, since a chain further ahead simply re-sees (and
    /// idempotently re-skips, in [`Self::apply_envelope`]) entries it has
    /// already applied.
    ///
    /// A mismatch means the outbox lost its own continuity (this being one
    /// shared outbox, that happens to every chain in it at once, not one at
    /// a time) - `seq` numbers from before the reset name a lineage that no
    /// longer exists. There is no rescan-from-height fallback to fall back
    /// to, so this does the next best thing: log loudly (this is the "page
    /// someone" moment, not a silent one), re-arm `watch_retry` for every
    /// chain that had a cursor, and resume from whatever the new outbox
    /// currently retains from its oldest entry. Anything paid entirely
    /// inside the gap is not recovered by this - only a payment still
    /// pending when the gap closes is.
    async fn reconcile_cursors(
        &self,
        cursors: &mut HashMap<u64, ChainCursor>,
        bridge_epoch: i64,
    ) -> Option<EventCursor> {
        if cursors.is_empty() {
            // Never resumed before: nothing stored to lose, so whatever the
            // outbox currently retains from its start is a strict gain.
            return None;
        }

        let stored_epoch = cursors.values().next().map(|c| c.epoch);
        if stored_epoch == Some(bridge_epoch) {
            // `unwrap_or_default` rather than `expect`: `cursors` was
            // checked non-empty above, so `0` here is unreachable, not a
            // real fallback.
            let min_seq = cursors.values().map(|c| c.seq).min().unwrap_or_default();
            return Some(EventCursor {
                epoch: bridge_epoch,
                seq: min_seq,
                block_height: 0,
            });
        }

        tracing::error!(
            ?stored_epoch,
            bridge_epoch,
            "event outbox epoch changed since this server last resumed; the gap since its last \
             commit cannot be replayed. Re-arming watch_retry for every watch on the affected \
             chains and resuming from the outbox's oldest retained event."
        );
        self.break_lineage(cursors).await;
        None
    }

    /// Re-arm `watch_retry` for every chain this server had a cursor for,
    /// then forget those cursors.
    ///
    /// Shared by [`Self::reconcile_cursors`] (the stored epoch no longer
    /// matches the outbox's) and [`Self::run`]'s resume loop (the outbox
    /// reported the requested position as trimmed, which
    /// [`evm::monitor::bridge::EventBridge::subscribe_from`] already turned
    /// into a fresh epoch on its side) - both mean the gap since the last
    /// commit cannot be replayed, and re-driving every live watch within
    /// `watch_retry`'s normal cycle is the best available recovery.
    async fn break_lineage(&self, cursors: &mut HashMap<u64, ChainCursor>) {
        let chain_ids: Vec<u64> = cursors.keys().copied().collect();
        for chain_id in chain_ids {
            if let Err(e) = self
                .data_service
                .reset_chain_watch_notifications(chain_id)
                .await
            {
                tracing::error!(
                    chain_id,
                    error = %e,
                    "failed to re-arm watch_retry after an event outbox lineage break"
                );
            }
        }
        cursors.clear();
    }

    /// Apply one envelope and, only once it has been applied, durably
    /// commit having done so.
    ///
    /// Ordering matters: a crash between "applied" and "committed" is fine,
    /// since delivery is at-least-once and the apply is idempotent, but
    /// committing first and then crashing before applying would silently
    /// lose the event - the exact failure this whole mechanism exists to
    /// close.
    async fn apply_envelope(
        &self,
        envelope: EventEnvelope,
        cursors: &mut HashMap<u64, ChainCursor>,
    ) {
        let chain_id = envelope.chain_id;

        if let Some(applied) = cursors.get(&chain_id)
            && envelope.cursor.epoch == applied.epoch
            && envelope.cursor.seq <= applied.seq
        {
            // Already applied. Reachable because resume uses one shared
            // low-water mark across chains: a chain further ahead than the
            // slowest one sees its own already-applied entries again.
            return;
        }

        if let Err(e) = self.handle_event(envelope.event).await {
            tracing::error!(error = %e, "Failed to handle event");
            return;
        }

        let cursor = ChainCursor {
            epoch: envelope.cursor.epoch,
            seq: envelope.cursor.seq,
            block_height: envelope.cursor.block_height,
        };
        if let Err(e) = self
            .data_service
            .commit_chain_cursor(ADAPTER_ID, chain_id, cursor)
            .await
        {
            tracing::error!(chain_id, error = %e, "failed to commit chain cursor");
            return;
        }
        cursors.insert(chain_id, cursor);
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
