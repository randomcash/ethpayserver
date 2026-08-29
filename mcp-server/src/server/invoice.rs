//! Invoice creation and retrieval tool implementations.

use chrono::Utc;
use rust_decimal::Decimal;
use uuid::Uuid;

use evm::XpubDeriver;
use rates::is_fiat_currency;
use types::currency::DEFAULT_INVOICE_EXPIRATION_SECS;
use types::{
    InvoiceId, InvoiceQueryParams, InvoiceReader, InvoiceStatus, InvoiceWriter, PaymentMethodId,
    PaymentOptionData, PaymentOptionId, PaymentOptionReader, PaymentOptionWriter, StoreId,
    StorePaymentMethodReader, StorePaymentMethodWriter, WatchedAddressWriter, traits::InvoiceData,
};

use super::EthpayMcpServer;
use super::args::{CreateInvoiceArgs, GetInvoiceArgs, ListInvoicesArgs};
use super::convert::{convert_human_to_smallest_unit, convert_to_crypto_smallest_unit};

impl EthpayMcpServer {
    pub(super) async fn do_create_invoice(
        &self,
        args: CreateInvoiceArgs,
    ) -> Result<String, String> {
        let store_uuid: Uuid = args.store_id.parse().map_err(|_| "Invalid store_id UUID")?;
        let store_id = StoreId(store_uuid);
        self.authorize_store(store_id)?;

        // Get enabled payment methods for the store
        let payment_methods =
            StorePaymentMethodReader::get_enabled_payment_methods(&*self.data_service, store_uuid)
                .await
                .map_err(|e| format!("Failed to get payment methods: {e}"))?;

        if payment_methods.is_empty() {
            return Err("Store has no enabled payment methods".into());
        }

        // Pre-validate: fetch rates for cross-currency invoices
        let invoice_currency_upper = args.currency.to_uppercase();
        let mut validated_methods: Vec<(
            usize,
            String,
            Option<String>,
            Option<chrono::DateTime<Utc>>,
        )> = Vec::new();

        for (idx, pm) in payment_methods.iter().enumerate() {
            let asset_upper = pm.asset_symbol.to_uppercase();

            if invoice_currency_upper == asset_upper {
                // Same asset — no rate conversion
                if is_fiat_currency(&args.currency) {
                    continue;
                }
                let amount = convert_human_to_smallest_unit(&args.amount, pm.decimals)?;
                validated_methods.push((idx, amount, None, None));
            } else {
                // Cross-currency �� need exchange rate
                match self
                    .rate_provider
                    .get_rate(&args.currency, &pm.asset_symbol)
                    .await
                {
                    Ok(rate) => {
                        if rate.rate <= Decimal::ZERO {
                            return Err(format!(
                                "Non-positive exchange rate for {}/{}",
                                args.currency, pm.asset_symbol
                            ));
                        }
                        let amount =
                            convert_to_crypto_smallest_unit(&args.amount, rate.rate, pm.decimals)?;
                        validated_methods.push((
                            idx,
                            amount,
                            Some(rate.rate.to_string()),
                            Some(rate.timestamp),
                        ));
                    }
                    Err(rates::RateError::UnsupportedPair { .. }) => continue,
                    Err(e) => return Err(format!("Rate fetch failed: {e}")),
                }
            }
        }

        if validated_methods.is_empty() {
            return Err("No payment methods with supported rate pairs for this currency".into());
        }

        let expiration_secs = args
            .expiration_seconds
            .unwrap_or(DEFAULT_INVOICE_EXPIRATION_SECS);
        let expires_at = Utc::now() + chrono::Duration::seconds(expiration_secs as i64);

        let metadata = match (args.customer_email, args.metadata) {
            (Some(email), Some(mut meta)) => {
                if let Some(obj) = meta.as_object_mut() {
                    obj.entry("customer_email")
                        .or_insert_with(|| serde_json::Value::String(email));
                }
                Some(meta)
            }
            (Some(email), None) => Some(serde_json::json!({ "customer_email": email })),
            (None, meta) => meta,
        };

        let invoice = InvoiceData {
            id: InvoiceId::new(),
            store_id,
            currency: args.currency.clone(),
            status: InvoiceStatus::Pending,
            amount: args.amount.clone(),
            amount_received: "0".to_string(),
            created_at: Utc::now(),
            expires_at,
            metadata,
            extra: None,
        };

        InvoiceWriter::upsert(&*self.data_service, &invoice)
            .await
            .map_err(|e| format!("Failed to create invoice: {e}"))?;

        // Create payment options
        let mut options_json = Vec::new();

        for (method_idx, crypto_amount, rate_str, rate_at) in validated_methods {
            let pm = &payment_methods[method_idx];

            let index = StorePaymentMethodWriter::next_derivation_index(&*self.data_service, pm.id)
                .await
                .map_err(|e| format!("Failed to get derivation index: {e}"))?;

            let deriver =
                XpubDeriver::from_xpub(&pm.xpub).map_err(|e| format!("Invalid xpub: {e}"))?;
            let address = deriver
                .derive_address(index as u32)
                .map_err(|e| format!("Address derivation failed: {e}"))?;
            let payment_address = address.to_string();

            let option = PaymentOptionData {
                id: PaymentOptionId(Uuid::new_v4()),
                invoice_id: invoice.id.clone(),
                payment_method_id: PaymentMethodId::new(&pm.asset_symbol, pm.chain_id),
                chain_id: pm.chain_id,
                asset_symbol: pm.asset_symbol.clone(),
                token_address: pm.token_address.clone(),
                decimals: pm.decimals,
                payment_address: payment_address.clone(),
                amount: crypto_amount,
                rate: rate_str,
                rate_at,
                is_active: true,
                created_at: Utc::now(),
            };

            PaymentOptionWriter::create(&*self.data_service, &option)
                .await
                .map_err(|e| format!("Failed to create payment option: {e}"))?;

            // Save watched address to database
            let token_addr_str = pm.token_address.as_deref();
            WatchedAddressWriter::upsert(
                &*self.data_service,
                &payment_address,
                &option.id,
                pm.chain_id,
                token_addr_str,
            )
            .await
            .map_err(|e| format!("Failed to save watched address: {e}"))?;

            // Notify EVM monitor if available
            if let Some(ref monitor) = self.evm_monitor
                && let Ok(invoice_uuid) = Uuid::parse_str(&invoice.id.0)
            {
                let expected = option.amount.parse::<evm::U256>().ok();
                let token_contract: Option<evm::Address> =
                    pm.token_address.as_ref().and_then(|a| a.parse().ok());

                let cmd = evm::monitor::events::MonitorCommand::WatchAddress(
                    evm::monitor::events::WatchAddressCommand {
                        chain_id: pm.chain_id,
                        address,
                        invoice_id: invoice_uuid,
                        expected_amount: expected,
                        token_contract,
                    },
                );
                if let Err(e) = monitor.publish_command(&cmd).await {
                    tracing::warn!(
                        invoice_id = %invoice.id.0,
                        address = %payment_address,
                        error = %e,
                        "Failed to send WatchAddress command, will be retried"
                    );
                } else if let Err(e) = WatchedAddressWriter::mark_notified(
                    &*self.data_service,
                    &payment_address,
                    pm.chain_id,
                    token_addr_str,
                )
                .await
                {
                    tracing::warn!(error = %e, "Failed to mark watch as notified");
                }
            }

            options_json.push(serde_json::json!({
                "id": option.id.0.to_string(),
                "payment_method_id": option.payment_method_id.0,
                "chain_id": option.chain_id,
                "asset_symbol": option.asset_symbol,
                "token_address": option.token_address,
                "decimals": option.decimals,
                "payment_address": option.payment_address,
                "amount": option.amount,
                "rate": option.rate,
                "is_active": option.is_active,
            }));
        }

        let result = serde_json::json!({
            "id": invoice.id.0,
            "currency": invoice.currency,
            "status": invoice.status.to_string(),
            "amount": invoice.amount,
            "amount_received": invoice.amount_received,
            "created_at": invoice.created_at.to_rfc3339(),
            "expires_at": invoice.expires_at.to_rfc3339(),
            "payment_options": options_json,
        });

        Ok(serde_json::to_string_pretty(&result).unwrap())
    }

    pub(super) async fn do_get_invoice(&self, args: GetInvoiceArgs) -> Result<String, String> {
        let id = InvoiceId::from_string(args.invoice_id);
        let invoice = self.authorize_invoice(&id).await?;

        let options = PaymentOptionReader::get_for_invoice(&*self.data_service, &id)
            .await
            .unwrap_or_default();

        let result = serde_json::json!({
            "id": invoice.id.0,
            "store_id": invoice.store_id.0.to_string(),
            "currency": invoice.currency,
            "status": invoice.status.to_string(),
            "amount": invoice.amount,
            "amount_received": invoice.amount_received,
            "created_at": invoice.created_at.to_rfc3339(),
            "expires_at": invoice.expires_at.to_rfc3339(),
            "metadata": invoice.metadata,
            "payment_options": options.iter().map(|o| serde_json::json!({
                "id": o.id.0.to_string(),
                "payment_method_id": o.payment_method_id.0,
                "chain_id": o.chain_id,
                "asset_symbol": o.asset_symbol,
                "token_address": o.token_address,
                "decimals": o.decimals,
                "payment_address": o.payment_address,
                "amount": o.amount,
                "rate": o.rate,
                "is_active": o.is_active,
            })).collect::<Vec<_>>(),
        });

        Ok(serde_json::to_string_pretty(&result).unwrap())
    }

    pub(super) async fn do_list_invoices(&self, args: ListInvoicesArgs) -> Result<String, String> {
        let store_uuid: Uuid = args.store_id.parse().map_err(|_| "Invalid store_id UUID")?;
        self.authorize_store(StoreId(store_uuid))?;

        let mut params = InvoiceQueryParams::new().with_store_id(StoreId(store_uuid));

        if let Some(status) = args.status {
            let status: InvoiceStatus = status.parse().map_err(|_| "Invalid status filter")?;
            params = params.with_status(status);
        }
        if let Some(currency) = args.currency {
            params = params.with_currency(currency);
        }
        if let Some(limit) = args.limit {
            params = params.with_limit(limit);
        }
        if let Some(offset) = args.offset {
            params = params.with_offset(offset);
        }

        let (total, invoices) = InvoiceReader::query(&*self.data_service, &params)
            .await
            .map_err(|e| format!("Query failed: {e}"))?;

        let result = serde_json::json!({
            "total": total,
            "invoices": invoices.iter().map(|inv| serde_json::json!({
                "id": inv.id.0,
                "currency": inv.currency,
                "status": inv.status.to_string(),
                "amount": inv.amount,
                "amount_received": inv.amount_received,
                "created_at": inv.created_at.to_rfc3339(),
                "expires_at": inv.expires_at.to_rfc3339(),
            })).collect::<Vec<_>>(),
        });

        Ok(serde_json::to_string_pretty(&result).unwrap())
    }
}
