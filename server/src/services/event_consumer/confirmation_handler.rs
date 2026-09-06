//! Handler for `PaymentConfirmed` events and customer receipt emails.

use auth::StoreRepository;
use chrono::Utc;
use evm::get_any_chain_config;
use evm::monitor::events::PaymentConfirmed;
use rust_decimal::Decimal;
use types::{
    InvoiceData, InvoiceId, InvoiceReader, InvoiceStatus, InvoiceWriter, PaymentData,
    PaymentReader, PaymentWriter, StoreSettingsReader,
};

use crate::api::ws::StatusUpdate;
use crate::metrics;
use crate::services::email::ReceiptData;
use crate::services::evm_monitor::EVMMonitor;
use crate::services::webhook::WebhookDataService;
use crate::services::webhook::WebhookEventType;

use super::{EventConsumer, EventConsumerDataService, EventConsumerError};

impl<
    D: EventConsumerDataService + 'static,
    M: EVMMonitor + 'static,
    W: WebhookDataService + 'static,
> EventConsumer<D, M, W>
{
    /// Handle PaymentConfirmed event.
    ///
    /// Updates payment confirmation status and transitions invoice to `paid`
    /// if amount_received >= amount.
    #[allow(clippy::too_many_lines, clippy::cognitive_complexity)] // confirmation state machine — reads/updates/transitions in one flow
    pub(super) async fn handle_payment_confirmed(
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

                    // Send customer receipt email (best-effort, never blocks payment flow)
                    self.send_customer_receipt(&invoice, payment, event.chain_id)
                        .await;
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

    /// Send a payment receipt email to the customer (best-effort).
    ///
    /// Checks: customer_email in metadata, store's customer_receipts_enabled toggle.
    /// Errors are logged but never propagated — email must not block the payment flow.
    pub(super) async fn send_customer_receipt(
        &self,
        invoice: &InvoiceData,
        payment: &PaymentData,
        chain_id: u64,
    ) {
        let Some(email) = Self::extract_customer_email(invoice) else {
            return;
        };

        if self.receipts_disabled_for_store(invoice.store_id.0).await {
            return;
        }

        let merchant_name = match StoreRepository::get_store(
            &*self.data_service,
            types::StoreId(invoice.store_id.0),
        )
        .await
        {
            Ok(Some(store)) => store.name,
            _ => "Merchant".to_string(),
        };

        let explorer_url = get_any_chain_config(chain_id)
            .map(|cfg| cfg.tx_explorer_url(&payment.tx_hash))
            .unwrap_or_else(|| payment.tx_hash.clone());

        let receipt = ReceiptData {
            invoice_id: invoice.id.as_str().to_string(),
            amount: invoice.amount.clone(),
            currency: invoice.currency.clone(),
            tx_hash: payment.tx_hash.clone(),
            explorer_url,
            paid_at: payment.confirmed_at.unwrap_or_else(Utc::now),
            merchant_name,
        };

        if let Err(e) = self.email_sender.send_receipt(email, &receipt).await {
            tracing::warn!(
                invoice_id = %invoice.id.as_str(),
                customer_email = %email,
                error = %e,
                "Failed to send customer receipt email"
            );
        } else {
            tracing::info!(
                invoice_id = %invoice.id.as_str(),
                customer_email = %email,
                "Customer receipt email sent"
            );
        }
    }

    /// Extract customer email from invoice metadata.
    /// The address a receipt goes to.
    ///
    /// Prefers the dedicated column, falling back to `metadata` for invoices
    /// created before it existed (RCS-215). The fallback is not decoration: new
    /// writes populate the column and no longer put the address in metadata, so
    /// reading metadata alone would have stopped receipts for every new invoice
    /// - silently, since a missing address is a normal, unlogged case here.
    pub(super) fn extract_customer_email(invoice: &InvoiceData) -> Option<&str> {
        invoice.customer_email.as_deref().or_else(|| {
            invoice
                .metadata
                .as_ref()
                .and_then(|m| m.get("customer_email").or_else(|| m.get("buyer_email")))
                .and_then(|v| v.as_str())
        })
    }

    /// Check if customer receipts are disabled for the given store.
    pub(super) async fn receipts_disabled_for_store(&self, store_id: uuid::Uuid) -> bool {
        if let Ok(Some(settings)) =
            StoreSettingsReader::get_store_settings(&*self.data_service, store_id).await
            && settings.notification_prefs.get("customer_receipts_enabled")
                == Some(&serde_json::Value::Bool(false))
        {
            tracing::trace!(
                store_id = %store_id,
                "Customer receipts disabled for store"
            );
            return true;
        }
        false
    }
}
