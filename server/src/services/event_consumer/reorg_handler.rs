//! Handler for `ReorgDetected` events.

use evm::chain_id_to_network;
use evm::monitor::events::ReorgDetected;
use types::{InvoiceId, InvoiceStatus, InvoiceWriter, PaymentReader};

use crate::api::ws::StatusUpdate;
use crate::services::evm_monitor::EVMMonitor;
use crate::services::webhook::WebhookDataService;

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
    #[allow(clippy::cognitive_complexity)]
    pub(super) async fn handle_reorg_detected(
        &self,
        event: ReorgDetected,
    ) -> Result<(), EventConsumerError> {
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
}
