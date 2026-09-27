//! Drains the transactional outbox of webhook notification obligations.
//!
//! `payment_handler` records an obligation in the same database transaction
//! as the payment row it is about (see `data_service::webhook_outbox` for
//! why), so a crash between "payment committed" and "obligation recorded"
//! cannot happen. This is the consumer side: nothing else in this codebase
//! reads `webhook_outbox`, so this is the only place an obligation ever moves
//! from "recorded" to "delivery attempted". Polling on a timer rather than
//! reacting inline to the write means a slow or unavailable Redis cannot
//! block the event consumer racing to keep up with the chain.
//!
//! This server typically runs more than one instance for availability, so
//! each poll claims its batch (`WebhookOutboxReader::claim_undispatched_obligations`)
//! rather than merely reading it - a plain read would let two instances'
//! timers both pick up the same obligation and queue the same webhook twice.

use std::sync::Arc;
use std::time::Duration;

use data_service::{WebhookObligation, WebhookOutboxReader, WebhookOutboxWriter};
use tokio::time::interval;
use types::{InvoiceId, InvoiceReader, PaymentReader, StoreSettingsReader, StoreWebhookReader};
use uuid::Uuid;

use crate::services::webhook::{
    QueueOutcome, WebhookEventType, WebhookPayload, WebhookSink, queue_for_store,
};

/// Trait for data service requirements in `WebhookOutboxDrainService`.
pub trait OutboxDrainDataService:
    WebhookOutboxReader
    + WebhookOutboxWriter
    + PaymentReader
    + InvoiceReader
    + StoreWebhookReader
    + StoreSettingsReader
    + Send
    + Sync
{
}

impl<T> OutboxDrainDataService for T where
    T: WebhookOutboxReader
        + WebhookOutboxWriter
        + PaymentReader
        + InvoiceReader
        + StoreWebhookReader
        + StoreSettingsReader
        + Send
        + Sync
{
}

/// Configuration for the webhook outbox drain service.
#[derive(Debug, Clone)]
pub struct WebhookOutboxDrainConfig {
    /// Interval between drain passes.
    pub poll_interval: Duration,
    /// Maximum obligations read in one pass, so a large backlog cannot make
    /// a single pass run unboundedly long.
    pub batch_size: i64,
    /// How long a claimed obligation stays invisible to another drain
    /// instance's claim before it is treated as abandoned and reclaimed.
    ///
    /// Must comfortably exceed how long a real dispatch attempt takes (an
    /// invoice/payment lookup plus handing the job to the sink), or a
    /// still-in-flight obligation would be reclaimed and dispatched twice by
    /// another instance while the first is still working on it.
    pub claim_visibility: Duration,
}

impl Default for WebhookOutboxDrainConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(5),
            batch_size: 100,
            claim_visibility: Duration::from_secs(30),
        }
    }
}

impl WebhookOutboxDrainConfig {
    /// Load configuration from environment variables.
    ///
    /// - `WEBHOOK_OUTBOX_DRAIN_INTERVAL_SECS` - poll interval in seconds (default: 5)
    /// - `WEBHOOK_OUTBOX_DRAIN_BATCH_SIZE` - obligations read per pass (default: 100)
    /// - `WEBHOOK_OUTBOX_CLAIM_VISIBILITY_SECS` - claimed-obligation visibility timeout (default: 30)
    pub fn from_env() -> Self {
        Self {
            poll_interval: Duration::from_secs(
                std::env::var("WEBHOOK_OUTBOX_DRAIN_INTERVAL_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(5),
            ),
            batch_size: std::env::var("WEBHOOK_OUTBOX_DRAIN_BATCH_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100),
            claim_visibility: Duration::from_secs(
                std::env::var("WEBHOOK_OUTBOX_CLAIM_VISIBILITY_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(30),
            ),
        }
    }
}

/// What to do next after trying to load a row an obligation refers to.
enum Lookup<T> {
    Found(T),
    /// The row is gone for good (or never existed) - a read will never
    /// succeed, so give up on this obligation rather than retry it forever.
    GiveUp,
    /// A transient read failure. Leave the obligation undispatched so the
    /// next drain pass tries again.
    Retry,
}

/// Background service that turns undispatched webhook obligations into
/// actual queued deliveries.
pub struct WebhookOutboxDrainService<D: OutboxDrainDataService> {
    data_service: Arc<D>,
    sink: Arc<dyn WebhookSink>,
    config: WebhookOutboxDrainConfig,
}

impl<D: OutboxDrainDataService + 'static> WebhookOutboxDrainService<D> {
    /// Create a new outbox drain service.
    pub fn new(
        data_service: Arc<D>,
        sink: Arc<dyn WebhookSink>,
        config: WebhookOutboxDrainConfig,
    ) -> Self {
        Self {
            data_service,
            sink,
            config,
        }
    }

    /// Run the drain service as a background task.
    pub async fn run(self) {
        tracing::info!(
            poll_interval_secs = self.config.poll_interval.as_secs(),
            "Starting webhook outbox drain service"
        );

        let mut ticker = interval(self.config.poll_interval);

        loop {
            ticker.tick().await;
            self.drain_once().await;
        }
    }

    /// Claim one batch of undispatched obligations and dispatch each.
    async fn drain_once(&self) {
        let obligations = match self
            .data_service
            .claim_undispatched_obligations(
                self.config.batch_size,
                self.config.claim_visibility.as_secs() as i64,
            )
            .await
        {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to read webhook outbox");
                return;
            }
        };

        for obligation in obligations {
            self.dispatch_obligation(obligation).await;
        }
    }

    /// Convert one obligation into an actual queued delivery, or leave it
    /// undispatched so the next pass retries it.
    async fn dispatch_obligation(&self, obligation: WebhookObligation) {
        let Some(event_type) = parse_event_type(&obligation.event_type) else {
            tracing::error!(
                obligation_id = %obligation.id,
                event_type = %obligation.event_type,
                "Unrecognized webhook event type in outbox; marking dispatched rather than retrying forever"
            );
            self.mark_dispatched(obligation.id).await;
            return;
        };

        let invoice = match self.load_invoice(&obligation).await {
            Lookup::Found(invoice) => invoice,
            Lookup::GiveUp => {
                self.mark_dispatched(obligation.id).await;
                return;
            }
            Lookup::Retry => return,
        };

        let payment = match self.load_payment(&obligation).await {
            Lookup::Found(payment) => payment,
            Lookup::GiveUp => {
                self.mark_dispatched(obligation.id).await;
                return;
            }
            Lookup::Retry => return,
        };

        let payload = WebhookPayload::payment_event(event_type, &invoice, &payment);
        match queue_for_store(
            self.sink.as_ref(),
            &*self.data_service,
            invoice.store_id.0,
            payload,
        )
        .await
        {
            QueueOutcome::Queued | QueueOutcome::Skipped => {
                self.mark_dispatched(obligation.id).await;
            }
            QueueOutcome::Failed => {
                tracing::warn!(
                    obligation_id = %obligation.id,
                    "Failed to queue webhook for outbox obligation; will retry"
                );
            }
        }
    }

    /// Look up the invoice an obligation is about.
    async fn load_invoice(&self, obligation: &WebhookObligation) -> Lookup<types::InvoiceData> {
        let invoice_id = InvoiceId::from_string(obligation.invoice_id.clone());
        match InvoiceReader::get(&*self.data_service, &invoice_id).await {
            Ok(Some(invoice)) => Lookup::Found(invoice),
            Ok(None) => {
                // Invoices are never deleted in this system, so a missing
                // invoice here means the obligation outlived its subject -
                // there is nothing left to notify about, and retrying cannot
                // change that.
                tracing::error!(
                    obligation_id = %obligation.id,
                    invoice_id = %obligation.invoice_id,
                    "Webhook obligation's invoice no longer exists; marking dispatched, notification lost"
                );
                Lookup::GiveUp
            }
            Err(e) => {
                tracing::warn!(
                    obligation_id = %obligation.id,
                    error = %e,
                    "Failed to load invoice for webhook obligation; will retry"
                );
                Lookup::Retry
            }
        }
    }

    /// Look up the payment an obligation is about.
    async fn load_payment(&self, obligation: &WebhookObligation) -> Lookup<types::PaymentData> {
        match PaymentReader::get(&*self.data_service, obligation.payment_id).await {
            Ok(Some(payment)) => Lookup::Found(payment),
            Ok(None) => {
                // `payment_id` carries `ON DELETE CASCADE`, so the outbox row
                // would have been deleted along with its payment - this
                // branch should be unreachable, logged loudly if it isn't.
                tracing::error!(
                    obligation_id = %obligation.id,
                    payment_id = %obligation.payment_id,
                    "Webhook obligation's payment no longer exists; marking dispatched, notification lost"
                );
                Lookup::GiveUp
            }
            Err(e) => {
                tracing::warn!(
                    obligation_id = %obligation.id,
                    error = %e,
                    "Failed to load payment for webhook obligation; will retry"
                );
                Lookup::Retry
            }
        }
    }

    async fn mark_dispatched(&self, id: Uuid) {
        if let Err(e) = self.data_service.mark_obligation_dispatched(id).await {
            tracing::warn!(
                obligation_id = %id,
                error = %e,
                "Failed to mark webhook obligation dispatched; may be redelivered"
            );
        }
    }
}

/// Parse an outbox row's `event_type` column back into the enum, given only
/// the wire name ([`WebhookEventType::as_str`]) it was stored as. Its
/// `Deserialize` derive (`#[serde(rename_all = "snake_case")]`) already
/// matches that name exactly, so this reuses it rather than hand-listing the
/// variants a second time.
fn parse_event_type(s: &str) -> Option<WebhookEventType> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::Mutex;

    use async_trait::async_trait;
    use chrono::Utc;
    use data_service::InMemoryDataService;
    use types::{InvoiceId, InvoiceWriter, StoreId};

    use super::*;
    use crate::services::webhook::{WebhookError, WebhookJob};

    /// A [`WebhookSink`] that records every job handed to it instead of
    /// touching Redis, so the drain's decisions can be asserted directly
    /// (see `WebhookSink`'s own doc comment for why this seam exists).
    #[derive(Default)]
    struct RecordingSink {
        jobs: Mutex<Vec<WebhookJob>>,
    }

    #[async_trait]
    impl WebhookSink for RecordingSink {
        async fn queue(&self, job: WebhookJob) -> Result<(), WebhookError> {
            self.jobs.lock().unwrap().push(job);
            Ok(())
        }
    }

    fn test_invoice(store_id: StoreId) -> types::InvoiceData {
        types::InvoiceData {
            id: InvoiceId::from_string("inv_outbox_drain".to_string()),
            store_id,
            currency: "ETH".to_string(),
            status: types::InvoiceStatus::Processing,
            amount: "1000".to_string(),
            amount_received: "1000".to_string(),
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            metadata: None,
            customer_email: None,
            extra: None,
        }
    }

    fn test_payment(invoice_id: &InvoiceId) -> types::PaymentData {
        types::PaymentData {
            id: Uuid::new_v4(),
            invoice_id: invoice_id.clone(),
            payment_option_id: Some(Uuid::new_v4()),
            chain_id: types::ChainId::evm(1),
            asset_type: types::AssetType::Native,
            amount: "1000".to_string(),
            asset_symbol: "ETH".to_string(),
            token_address: None,
            tx_hash: "0xabc".to_string(),
            block_number: Some(1),
            detected_at: Utc::now(),
            confirmed_at: None,
            from_address: Some("0xdef".to_string()),
            reorged: false,
            extra: None,
            credited_amount: Some("1000".to_string()),
            rate_used: None,
            rate_applied_at: None,
        }
    }

    #[test]
    fn unrecognized_event_type_does_not_parse() {
        assert!(parse_event_type("not_a_real_event").is_none());
    }

    #[test]
    fn every_known_wire_name_round_trips() {
        for name in [
            "payment_detected",
            "payment_confirmed",
            "payment_reorged",
            "invoice_expired",
            "invoice_cancelled",
            "late_paid",
        ] {
            assert!(
                parse_event_type(name).is_some(),
                "known wire name {name} failed to parse"
            );
        }
    }

    /// The defect this regresses: an obligation whose invoice has an enabled
    /// webhook must be turned into an actual queued job and marked dispatched
    /// in the same pass, not left for a human to notice it never moved.
    #[tokio::test]
    async fn an_obligation_with_a_configured_webhook_is_queued_and_marked_dispatched() {
        let data_service = Arc::new(InMemoryDataService::new());
        let invoice = test_invoice(StoreId::new());
        InvoiceWriter::upsert(&*data_service, &invoice)
            .await
            .expect("insert invoice");
        data_service.set_webhook(invoice.store_id.0, "https://example.com/hook", "secret");

        let payment = test_payment(&invoice.id);
        data_service::PaymentTxIndexWriter::upsert_with_tx_index_and_obligation(
            &*data_service,
            &payment,
            0,
            WebhookEventType::PaymentDetected.as_str(),
        )
        .await
        .expect("obligation recorded");

        let sink = Arc::new(RecordingSink::default());
        let service = WebhookOutboxDrainService::new(
            Arc::clone(&data_service),
            Arc::clone(&sink) as Arc<dyn WebhookSink>,
            WebhookOutboxDrainConfig::default(),
        );

        service.drain_once().await;

        assert_eq!(sink.jobs.lock().unwrap().len(), 1, "job should be queued");
        let remaining = data_service
            .claim_undispatched_obligations(10, 30)
            .await
            .expect("read outbox");
        assert!(
            remaining.is_empty(),
            "dispatched obligation must not be read again"
        );
    }

    /// A store with no webhook configured has nothing to deliver - the
    /// obligation must still be marked dispatched, not retried forever.
    #[tokio::test]
    async fn an_obligation_for_a_store_with_no_webhook_is_marked_dispatched_without_queuing() {
        let data_service = Arc::new(InMemoryDataService::new());
        let invoice = test_invoice(StoreId::new());
        InvoiceWriter::upsert(&*data_service, &invoice)
            .await
            .expect("insert invoice");
        // Deliberately no `set_webhook` call.

        let payment = test_payment(&invoice.id);
        data_service::PaymentTxIndexWriter::upsert_with_tx_index_and_obligation(
            &*data_service,
            &payment,
            0,
            WebhookEventType::PaymentDetected.as_str(),
        )
        .await
        .expect("obligation recorded");

        let sink = Arc::new(RecordingSink::default());
        let service = WebhookOutboxDrainService::new(
            Arc::clone(&data_service),
            Arc::clone(&sink) as Arc<dyn WebhookSink>,
            WebhookOutboxDrainConfig::default(),
        );

        service.drain_once().await;

        assert!(
            sink.jobs.lock().unwrap().is_empty(),
            "nothing should be queued with no webhook configured"
        );
        let remaining = data_service
            .claim_undispatched_obligations(10, 30)
            .await
            .expect("read outbox");
        assert!(
            remaining.is_empty(),
            "an obligation with nothing to deliver must still be marked dispatched"
        );
    }

    /// A [`WebhookSink`] that always fails, standing in for a Redis that is
    /// down or refusing writes.
    struct FailingSink;

    #[async_trait]
    impl WebhookSink for FailingSink {
        async fn queue(&self, _job: WebhookJob) -> Result<(), WebhookError> {
            Err(WebhookError::Redis("simulated queue failure".to_string()))
        }
    }

    /// The one behavior standing between a transient failure and a lost
    /// notification: `QueueOutcome::Failed` must leave the obligation
    /// undispatched so the next drain tick retries it, not mark it done and
    /// let the failure disappear the same way the ticket's original
    /// swallowed `if let Ok(Some(...))` did.
    #[tokio::test]
    async fn a_failed_queue_attempt_leaves_the_obligation_undispatched_for_retry() {
        let data_service = Arc::new(InMemoryDataService::new());
        let invoice = test_invoice(StoreId::new());
        InvoiceWriter::upsert(&*data_service, &invoice)
            .await
            .expect("insert invoice");
        data_service.set_webhook(invoice.store_id.0, "https://example.com/hook", "secret");

        let payment = test_payment(&invoice.id);
        data_service::PaymentTxIndexWriter::upsert_with_tx_index_and_obligation(
            &*data_service,
            &payment,
            0,
            WebhookEventType::PaymentDetected.as_str(),
        )
        .await
        .expect("obligation recorded");

        let sink = Arc::new(FailingSink);
        // Zero visibility: the claim `drain_once` takes below expires the
        // instant it is set, so the assertion's own claim call - a stand-in
        // for the next drain tick - can immediately observe whether the
        // obligation is still there to retry, without an artificial sleep.
        let config = WebhookOutboxDrainConfig {
            claim_visibility: Duration::ZERO,
            ..WebhookOutboxDrainConfig::default()
        };
        let service = WebhookOutboxDrainService::new(
            Arc::clone(&data_service),
            sink as Arc<dyn WebhookSink>,
            config,
        );

        service.drain_once().await;

        let remaining = data_service
            .claim_undispatched_obligations(10, 0)
            .await
            .expect("read outbox");
        assert_eq!(
            remaining.len(),
            1,
            "a failed queue attempt must leave the obligation for the next drain tick, \
             not mark it dispatched and lose it"
        );
    }

    /// A transient failure reading the invoice a payment's obligation is
    /// about must retry, not silently disappear the same way the ticket's
    /// original unchecked `if let Ok(Some(...))` did.
    #[tokio::test]
    async fn a_transient_invoice_read_failure_leaves_the_obligation_for_retry() {
        let data_service = Arc::new(InMemoryDataService::new());
        let invoice = test_invoice(StoreId::new());
        InvoiceWriter::upsert(&*data_service, &invoice)
            .await
            .expect("insert invoice");
        data_service.set_webhook(invoice.store_id.0, "https://example.com/hook", "secret");

        let payment = test_payment(&invoice.id);
        data_service::PaymentTxIndexWriter::upsert_with_tx_index_and_obligation(
            &*data_service,
            &payment,
            0,
            WebhookEventType::PaymentDetected.as_str(),
        )
        .await
        .expect("obligation recorded");

        data_service.fail_invoice_reads();

        let sink = Arc::new(RecordingSink::default());
        let config = WebhookOutboxDrainConfig {
            claim_visibility: Duration::ZERO,
            ..WebhookOutboxDrainConfig::default()
        };
        let service = WebhookOutboxDrainService::new(
            Arc::clone(&data_service),
            Arc::clone(&sink) as Arc<dyn WebhookSink>,
            config,
        );

        service.drain_once().await;

        assert!(
            sink.jobs.lock().unwrap().is_empty(),
            "nothing should be queued when the invoice lookup fails"
        );
        let remaining = data_service
            .claim_undispatched_obligations(10, 0)
            .await
            .expect("read outbox");
        assert_eq!(
            remaining.len(),
            1,
            "a transient invoice read failure must leave the obligation for the next \
             drain tick, not mark it dispatched and lose it"
        );
    }

    /// Same property as above, one lookup later: a transient failure reading
    /// the payment itself must also retry rather than lose the obligation.
    #[tokio::test]
    async fn a_transient_payment_read_failure_leaves_the_obligation_for_retry() {
        let data_service = Arc::new(InMemoryDataService::new());
        let invoice = test_invoice(StoreId::new());
        InvoiceWriter::upsert(&*data_service, &invoice)
            .await
            .expect("insert invoice");
        data_service.set_webhook(invoice.store_id.0, "https://example.com/hook", "secret");

        let payment = test_payment(&invoice.id);
        data_service::PaymentTxIndexWriter::upsert_with_tx_index_and_obligation(
            &*data_service,
            &payment,
            0,
            WebhookEventType::PaymentDetected.as_str(),
        )
        .await
        .expect("obligation recorded");

        data_service.fail_payment_reads();

        let sink = Arc::new(RecordingSink::default());
        let config = WebhookOutboxDrainConfig {
            claim_visibility: Duration::ZERO,
            ..WebhookOutboxDrainConfig::default()
        };
        let service = WebhookOutboxDrainService::new(
            Arc::clone(&data_service),
            Arc::clone(&sink) as Arc<dyn WebhookSink>,
            config,
        );

        service.drain_once().await;

        assert!(
            sink.jobs.lock().unwrap().is_empty(),
            "nothing should be queued when the payment lookup fails"
        );
        let remaining = data_service
            .claim_undispatched_obligations(10, 0)
            .await
            .expect("read outbox");
        assert_eq!(
            remaining.len(),
            1,
            "a transient payment read failure must leave the obligation for the next \
             drain tick, not mark it dispatched and lose it"
        );
    }

    /// Invoices are never deleted in this system, so this is believed
    /// unreachable in production - but the guard exists precisely to handle
    /// it if that belief is ever wrong, so it should behave as documented:
    /// give up rather than retry forever, since a missing invoice cannot
    /// become present.
    #[tokio::test]
    async fn an_obligation_whose_invoice_no_longer_exists_is_marked_dispatched_without_queuing() {
        let data_service = Arc::new(InMemoryDataService::new());
        let invoice = test_invoice(StoreId::new());
        // Deliberately never written: `upsert_with_tx_index_and_obligation`
        // only requires the payment, so this reproduces "obligation outlived
        // its invoice" without needing a delete path this double doesn't have.
        data_service.set_webhook(invoice.store_id.0, "https://example.com/hook", "secret");

        let payment = test_payment(&invoice.id);
        data_service::PaymentTxIndexWriter::upsert_with_tx_index_and_obligation(
            &*data_service,
            &payment,
            0,
            WebhookEventType::PaymentDetected.as_str(),
        )
        .await
        .expect("obligation recorded");

        let sink = Arc::new(RecordingSink::default());
        let service = WebhookOutboxDrainService::new(
            Arc::clone(&data_service),
            Arc::clone(&sink) as Arc<dyn WebhookSink>,
            WebhookOutboxDrainConfig::default(),
        );

        service.drain_once().await;

        assert!(
            sink.jobs.lock().unwrap().is_empty(),
            "nothing should be queued for an invoice that no longer exists"
        );
        let remaining = data_service
            .claim_undispatched_obligations(10, 30)
            .await
            .expect("read outbox");
        assert!(
            remaining.is_empty(),
            "an obligation whose invoice is gone for good must be marked dispatched, \
             not retried forever"
        );
    }
}
