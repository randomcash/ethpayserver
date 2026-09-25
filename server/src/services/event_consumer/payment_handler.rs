//! Handler for `PaymentDetected` events.

use chrono::Utc;
use data_service::{PaymentOptionReader, PaymentTxIndexWriter};
use evm::get_any_chain_config;
use evm::monitor::events::PaymentDetected;
use types::{AssetType, InvoiceId, InvoiceReader, PaymentData, TokenReader, WatchedAddressReader};
use uuid::Uuid;

use crate::api::ws::StatusUpdate;
use crate::metrics;
use crate::services::evm_monitor::EVMMonitor;
use crate::services::webhook::WebhookDataService;
use crate::services::webhook::WebhookEventType;

use super::{EventConsumer, EventConsumerDataService, EventConsumerError};

/// What to store in `payments.asset_symbol` for an ERC-20.
///
/// `ERC20` whenever the token has no symbol of its own - whether it is absent
/// from the `tokens` table or present without one. The two cases were handled
/// differently and only one of them was right: an unregistered token was
/// recorded as `0x1c7d4b...`, a truncated address.
///
/// That is a display string, and this column is not a display. It goes
/// wherever the column goes - the analytics reader groups by it, the plugin
/// volume capability prices against it - and nothing can resolve an
/// abbreviation. The first real USDC payment on testnet was therefore worth
/// zero to the billing ladder.
///
/// Generalising loses nothing. `token_address` sits on the same row and
/// carries the identity exactly; six hex digits never did.
#[must_use]
fn asset_symbol_for(symbol: Option<String>) -> String {
    match symbol {
        Some(symbol) if !symbol.trim().is_empty() => symbol,
        _ => "ERC20".to_string(),
    }
}

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
        // The monitor speaks EIP-155 numbers; everything past this boundary
        // speaks CAIP-2.
        let chain_id = types::ChainId::evm(event.chain_id);

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

            // Look the token up by chain. Every chain has an identifier now, so
            // the old "only if we recognise this network" branch is gone —
            // testnets used to fall through it and lose their token symbols.
            let symbol = {
                let found =
                    TokenReader::get_by_address(&*self.data_service, &chain_id, &token_addr_str)
                        .await?;
                if found.as_ref().is_none_or(|t| t.symbol.is_none()) {
                    tracing::warn!(
                        token_address = %token_addr_str,
                        chain_id = event.chain_id,
                        "no symbol for this token; recording it as ERC20 - register it in \
                         `tokens` to give this chain's payments a real one"
                    );
                }
                asset_symbol_for(found.and_then(|t| t.symbol))
            };

            (AssetType::ERC20, symbol, Some(token_addr_str))
        };

        // Look up the payment option by the payment address
        let payment_address_str = format!("{:#x}", event.payment_address);
        let payment_option_id = WatchedAddressReader::get_payment_option_id(
            &*self.data_service,
            &payment_address_str,
            &chain_id,
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
        metrics::record_payment_detected(&chain_id, &asset_symbol);

        let payment = PaymentData {
            id: Uuid::new_v4(),
            invoice_id: InvoiceId::from_string(event.invoice_id.to_string()),
            payment_option_id: payment_option_id.map(|id| id.0),
            chain_id: chain_id.clone(),
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

        // `tx_index` is the log index of this transfer within its transaction,
        // distinguishing two transfers batched into one tx (a multicall, an
        // exchange sweep) that would otherwise share (chain_id, tx_hash) and
        // collide in `unique_payment_tx`. Native transfers carry no log index
        // and get the sentinel -1 rather than 0: `check_native_payments` scans
        // only each transaction's top-level `to`/`value`, one entry per
        // tx_hash, so a fixed sentinel can never collide with another native
        // transfer in the same tx - but 0 is a real, reachable ERC20 log
        // index, and a contract that both receives ETH directly (top-level
        // `to`) and emits a Transfer log at index 0 in that same transaction
        // would otherwise collide two unrelated payments onto tx_index = 0.
        // Derived by `PaymentDetected::tx_index`, which branches on
        // `is_native` rather than on whether a log index is present. An ERC20
        // log that arrived without one is malformed, not native: filing it on
        // the -1 sentinel would merge it with a genuine native transfer in the
        // same transaction, which is precisely the collision the sentinel
        // exists to prevent. Rejected here the same way a missing
        // `token_address` already is.
        let tx_index = event.tx_index().ok_or_else(|| {
            EventConsumerError::InvalidData(format!(
                "ERC20 transfer in {:#x} has no log index; cannot tell it apart from \
                 other transfers in the same transaction",
                event.tx_hash
            ))
        })?;
        PaymentTxIndexWriter::upsert_with_tx_index(&*self.data_service, &payment, tx_index).await?;

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

#[cfg(test)]
mod symbol_tests {
    use super::asset_symbol_for;

    #[test]
    fn a_token_with_a_symbol_keeps_it() {
        assert_eq!(asset_symbol_for(Some("USDC".to_string())), "USDC");
    }

    /// The case that shipped wrong. An unregistered token was stored as
    /// `0x1c7d4b...`, which is a display string in a column that is read by
    /// machines: the analytics reader groups by it and the plugin volume
    /// capability asks a rate provider to price it. No provider can resolve
    /// an abbreviation, so a payment stored this way is worth nothing to the
    /// billing ladder - which is exactly what happened to the first USDC
    /// payment on testnet.
    #[test]
    fn an_unknown_token_is_never_recorded_as_an_abbreviated_address() {
        let symbol = asset_symbol_for(None);
        assert_eq!(symbol, "ERC20");
        assert!(
            !symbol.starts_with("0x"),
            "an address fragment is not a symbol: {symbol}"
        );
        assert!(
            !symbol.contains("..."),
            "an ellipsis means this was formatted for a screen: {symbol}"
        );
    }

    /// A row that exists but carries no symbol lands in the same place. The
    /// two used to diverge, and the divergence was the bug.
    #[test]
    fn a_registered_token_without_a_symbol_agrees_with_an_unregistered_one() {
        assert_eq!(
            asset_symbol_for(None),
            asset_symbol_for(Some(String::new()))
        );
        assert_eq!(
            asset_symbol_for(None),
            asset_symbol_for(Some("   ".to_string()))
        );
    }

    /// `asset_symbol` is `varchar(32)`. A full address is 42 characters,
    /// which is why the original truncated one - so the replacement has to
    /// fit without needing to.
    #[test]
    fn the_fallback_fits_the_column() {
        assert!(asset_symbol_for(None).len() <= 32);
    }
}
