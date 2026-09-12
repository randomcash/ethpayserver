//! Webhook notification dispatch for invoice events.

use types::{InvoiceData, PaymentData};

use crate::services::evm_monitor::EVMMonitor;
use crate::services::webhook::{
    WebhookDataService, WebhookEventType, WebhookPayload, queue_for_store,
};

use super::{EventConsumer, EventConsumerDataService};

impl<
    D: EventConsumerDataService + 'static,
    M: EVMMonitor + 'static,
    W: WebhookDataService + 'static,
> EventConsumer<D, M, W>
{
    /// Queue a webhook notification for an invoice status change.
    ///
    /// This is a non-blocking operation - errors are logged but don't stop event processing.
    pub(super) async fn queue_webhook(
        &self,
        event_type: WebhookEventType,
        invoice: &InvoiceData,
        payment: Option<&PaymentData>,
    ) {
        let payload = payment.map_or_else(
            || WebhookPayload::invoice_event(event_type, invoice),
            |p| WebhookPayload::payment_event(event_type, invoice, p),
        );
        self.queue_payload(invoice.store_id.0, payload).await;
    }

    /// Queue an already-built payload for the invoice's store.
    pub(super) async fn queue_payload(&self, store_id: uuid::Uuid, payload: WebhookPayload) {
        let Some(sink) = &self.webhook_service else {
            return;
        };

        queue_for_store(sink.as_ref(), &*self.data_service, store_id, payload).await;
    }
}
