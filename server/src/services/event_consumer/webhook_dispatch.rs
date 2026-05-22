//! Webhook notification dispatch for invoice events.

use evm::chain_id_to_network;
use types::{InvoiceData, PaymentData, StoreSettingsReader, StoreWebhookReader};
use uuid::Uuid;

use crate::services::evm_monitor::EVMMonitor;
use crate::services::webhook::{
    WebhookDataService, WebhookEventType, WebhookJob, WebhookPayload, WebhookPaymentInfo,
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
    #[allow(clippy::cognitive_complexity, clippy::too_many_lines)] // webhook payload assembly with optional fields
    pub(super) async fn queue_webhook(
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
}
