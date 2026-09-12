//! Handler for `ReorgDetected` events.

use std::collections::HashSet;

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
    /// Marks affected payments as reorged and reverts invoice status:
    /// - If other valid payments exist → `processing`
    /// - If no valid payments → `pending`
    ///
    /// Emits `payment_reorged` for each invoice whose payments were retracted.
    /// A subscriber was told about those payments and has no other way to
    /// learn they are gone: the invoice silently moving backwards is not a
    /// notification.
    #[allow(clippy::cognitive_complexity)]
    pub(super) async fn handle_reorg_detected(
        &self,
        event: ReorgDetected,
    ) -> Result<(), EventConsumerError> {
        let chain_id = types::ChainId::evm(event.chain_id);

        tracing::warn!(
            chain_id = %chain_id,
            fork_block = event.fork_block,
            depth = event.depth,
            affected_invoices = event.affected_invoices.len(),
            "Chain reorganization detected"
        );

        for invoice_uuid in &event.affected_invoices {
            let invoice_id = InvoiceId::from_string(invoice_uuid.to_string());

            // Which payments were already reorged before this event, so that
            // the retraction lists only what *this* reorg invalidated. Read
            // rather than re-derived: `mark_reorged` returns a count, and
            // restating its WHERE clause here would be a second copy of the
            // rule that could drift from the first.
            let already_reorged: HashSet<_> =
                PaymentReader::get_for_invoice(&*self.data_service, &invoice_id)
                    .await?
                    .into_iter()
                    .filter(|p| p.reorged)
                    .map(|p| p.id)
                    .collect();

            // Mark payments from this chain at or after the fork block as reorged
            let reorged_count = self
                .data_service
                .mark_reorged(&invoice_id, &chain_id, event.fork_block)
                .await?;

            if reorged_count == 0 {
                tracing::debug!(
                    invoice_id = %invoice_uuid,
                    fork_block = event.fork_block,
                    "No payments affected by reorg"
                );
                continue;
            }

            let retracted: Vec<PaymentData> =
                PaymentReader::get_for_invoice(&*self.data_service, &invoice_id)
                    .await?
                    .into_iter()
                    .filter(|p| p.reorged && !already_reorged.contains(&p.id))
                    .collect();

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

            self.notify_reorg(&invoice_id, &retracted).await;
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
