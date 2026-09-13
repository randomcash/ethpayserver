//! Handler for `ReorgDetected` events.

use std::collections::HashMap;

use evm::monitor::events::ReorgDetected;
use types::{InvoiceId, InvoiceReader, InvoiceStatus, InvoiceWriter, PaymentData, PaymentReader};

use crate::api::ws::StatusUpdate;
use crate::services::evm_monitor::EVMMonitor;
use crate::services::webhook::{WebhookDataService, WebhookPayload};

use super::{EventConsumer, EventConsumerDataService, EventConsumerError};

impl<
    D: EventConsumerDataService + 'static,
    M: EVMMonitor + 'static,
    W: WebhookDataService + 'static,
> EventConsumer<D, M, W>
{
    /// Handle ReorgDetected event.
    ///
    /// The candidate set comes from the database (`ReorgCandidateReader`),
    /// not `event.affected_invoices`. That list is the monitor's in-memory
    /// pending-payment map: it is empty right after a monitor restart, and it
    /// never includes a payment that has already confirmed and dropped out
    /// of that map. Either hole left an invoice stuck `paid` for money that
    /// had already been reorged away. The database is what survives both.
    ///
    /// `event.survived_tx_hashes` are transactions the monitor re-validated
    /// against the chain and found still present, merely relocated to a
    /// different block by the reorg. Those are excluded from retraction —
    /// retracting a payment that is genuinely still paid is the opposite
    /// error, and it un-pays an invoice a customer actually settled.
    ///
    /// Marks affected payments as reorged and reverts invoice status:
    /// - If other valid payments exist → `processing`
    /// - If no valid payments → `pending`
    ///
    /// Emits `payment_reorged` for each invoice whose payments were
    /// retracted. A subscriber was told about those payments and has no
    /// other way to learn they are gone: the invoice silently moving
    /// backwards is not a notification.
    #[allow(clippy::cognitive_complexity)]
    pub(super) async fn handle_reorg_detected(
        &self,
        event: ReorgDetected,
    ) -> Result<(), EventConsumerError> {
        let chain_id = types::ChainId::evm(event.chain_id);

        let candidates = self
            .data_service
            .reorg_candidates(&chain_id, event.fork_block)
            .await?;

        tracing::warn!(
            chain_id = %chain_id,
            fork_block = event.fork_block,
            depth = event.depth,
            candidates = candidates.len(),
            "Chain reorganization detected"
        );

        let mut by_invoice: HashMap<InvoiceId, Vec<PaymentData>> = HashMap::new();
        for payment in candidates {
            let survived = event
                .survived_tx_hashes
                .iter()
                .any(|hash| tx_hash_eq(hash, &payment.tx_hash));
            if survived {
                continue;
            }
            by_invoice
                .entry(payment.invoice_id.clone())
                .or_default()
                .push(payment);
        }

        for (invoice_id, mut to_retract) in by_invoice {
            for payment in &mut to_retract {
                self.data_service.mark_payment_reorged(payment.id).await?;
                // Mirror the write we just made, rather than re-reading: the
                // webhook payload below reports what this reorg just did.
                payment.reorged = true;
                payment.confirmed_at = None;
            }

            tracing::info!(
                invoice_id = %invoice_id,
                reorged_count = to_retract.len(),
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
                    invoice_id: invoice_id.as_str().to_string(),
                    status: new_status.to_string(),
                });
            }

            tracing::info!(
                invoice_id = %invoice_id,
                new_status = ?new_status,
                "Reverted invoice status after reorg"
            );

            self.notify_reorg(&invoice_id, &to_retract).await;
        }

        Ok(())
    }

    /// Tell the store's webhook subscriber to forget the retracted payments.
    ///
    /// The invoice is re-read so the payload reports the status and received
    /// amount the subscriber is being moved *to*, not the ones it already
    /// knows are wrong.
    async fn notify_reorg(&self, invoice_id: &InvoiceId, retracted: &[PaymentData]) {
        if retracted.is_empty() || self.webhook_service.is_none() {
            return;
        }

        let invoice = match InvoiceReader::get(&*self.data_service, invoice_id).await {
            Ok(Some(invoice)) => invoice,
            Ok(None) => {
                tracing::warn!(
                    invoice_id = %invoice_id.as_str(),
                    "Invoice vanished before the reorg webhook could be built"
                );
                return;
            }
            Err(e) => {
                tracing::warn!(
                    invoice_id = %invoice_id.as_str(),
                    error = %e,
                    "Failed to re-read invoice for the reorg webhook"
                );
                return;
            }
        };

        let store_id = invoice.store_id.0;
        let payload = WebhookPayload::payment_reorged(&invoice, retracted);
        self.queue_payload(store_id, payload).await;
    }
}

/// Whether `hash`, as the monitor observed it on chain, names the same
/// transaction as `stored`, as persisted in `payments.tx_hash`.
///
/// `payments.tx_hash` is written with this same `format!("{:#x}", …)` in
/// `payment_handler.rs`'s `handle_payment_detected`. If that write path ever
/// changes representation without a matching change here, every comparison
/// silently fails and this whole guard degrades to "nothing survived" with no
/// error — see `test_reorg_does_not_retract_a_survived_transaction`, which
/// pins the write path's literal output rather than re-deriving it with this
/// same call.
fn tx_hash_eq(hash: &evm::B256, stored: &str) -> bool {
    format!("{:#x}", hash) == stored
}
