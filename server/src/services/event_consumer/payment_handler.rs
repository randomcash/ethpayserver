//! Handler for `PaymentDetected` events.

use chrono::Utc;
use data_service::PaymentOptionReader;
use evm::monitor::events::PaymentDetected;
use evm::{chain_id_to_network, get_any_chain_config};
use types::{
    AssetType, InvoiceId, InvoiceReader, PaymentData, PaymentWriter, TokenReader,
    WatchedAddressReader,
};
use uuid::Uuid;

use crate::api::ws::StatusUpdate;
use crate::metrics;
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
    /// Handle PaymentDetected event.
    ///
    /// Creates a payment record in the database. The DB trigger automatically:
    /// - Updates invoice.amount_received
    /// - Transitions invoice status: pending → processing
    #[allow(clippy::too_many_lines, clippy::cognitive_complexity)] // payment state machine — single logical transaction
    pub(super) async fn handle_payment_detected(
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
}
