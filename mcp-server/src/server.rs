//! MCP server handler with tool implementations.

use std::sync::Arc;

use chrono::Utc;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{schemars, tool, tool_router};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use uuid::Uuid;

use auth::UserId;
use data_service::PgDataService;
use evm::XpubDeriver;
use evm::monitor::bridge::{COMMANDS_CHANNEL, EVENTS_CHANNEL, EventBridge, RedisBridge};
use rates::{RateProvider, is_fiat_currency};
use types::currency::DEFAULT_INVOICE_EXPIRATION_SECS;
use types::{
    InvoiceId, InvoiceQueryParams, InvoiceReader, InvoiceStatus, InvoiceWriter, PaymentMethodId,
    PaymentOptionData, PaymentOptionId, PaymentOptionReader, PaymentOptionWriter, PaymentReader,
    StoreId, StorePaymentMethodReader, StorePaymentMethodWriter, WatchedAddressWriter,
    traits::InvoiceData,
};

/// EVM monitor type (reuse the same Redis-backed monitor from the server crate).
pub type EvmMonitor = evm::monitor::bridge::RedisBridge;

/// Create an EVM monitor bridge from a Redis URL.
pub async fn create_evm_monitor(redis_url: &str) -> anyhow::Result<EvmMonitor> {
    let events_channel =
        std::env::var("REDIS_EVENTS_CHANNEL").unwrap_or_else(|_| EVENTS_CHANNEL.to_string());
    let commands_channel =
        std::env::var("REDIS_COMMANDS_CHANNEL").unwrap_or_else(|_| COMMANDS_CHANNEL.to_string());
    let bridge = RedisBridge::new(redis_url, &events_channel, &commands_channel).await?;
    Ok(bridge)
}

// ---------- Tool parameter types ----------

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateInvoiceArgs {
    #[schemars(description = "Store ID (UUID) to create the invoice for")]
    pub store_id: String,
    #[schemars(description = "Invoice currency (e.g. \"USD\", \"EUR\", \"ETH\")")]
    pub currency: String,
    #[schemars(
        description = "Amount in the currency's standard unit (e.g. \"100.00\" for USD, \"0.1\" for ETH)"
    )]
    pub amount: String,
    #[schemars(description = "Expiration in seconds from now (default: 900)")]
    pub expiration_seconds: Option<u64>,
    #[schemars(description = "Optional metadata as JSON object")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetInvoiceArgs {
    #[schemars(description = "Invoice ID to retrieve")]
    pub invoice_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListInvoicesArgs {
    #[schemars(description = "Store ID (UUID) to list invoices for")]
    pub store_id: String,
    #[schemars(
        description = "Filter by status: pending, processing, partially_paid, paid, expired, cancelled, refunded, late_paid"
    )]
    pub status: Option<String>,
    #[schemars(description = "Filter by currency (e.g. \"USD\")")]
    pub currency: Option<String>,
    #[schemars(description = "Maximum number of results (default: 50)")]
    pub limit: Option<i64>,
    #[schemars(description = "Offset for pagination (default: 0)")]
    pub offset: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetInvoicePaymentsArgs {
    #[schemars(description = "Invoice ID to get payments for")]
    pub invoice_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CancelInvoiceArgs {
    #[schemars(
        description = "Invoice ID to cancel (must be pending, processing, or partially_paid)"
    )]
    pub invoice_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetPaymentStatusArgs {
    #[schemars(description = "Invoice ID to check payment status for")]
    pub invoice_id: String,
}

// ---------- Server ----------

#[derive(Clone)]
pub struct EthpayMcpServer {
    data_service: Arc<PgDataService>,
    user_id: UserId,
    store_ids: Vec<StoreId>,
    rate_provider: Arc<dyn RateProvider>,
    evm_monitor: Option<Arc<EvmMonitor>>,
}

impl EthpayMcpServer {
    pub fn new(
        data_service: Arc<PgDataService>,
        user_id: UserId,
        store_ids: Vec<StoreId>,
        rate_provider: Arc<dyn RateProvider>,
        evm_monitor: Option<Arc<EvmMonitor>>,
    ) -> Self {
        Self {
            data_service,
            user_id,
            store_ids,
            rate_provider,
            evm_monitor,
        }
    }

    /// Check if the authenticated user has access to a store.
    fn authorize_store(&self, store_id: StoreId) -> Result<(), String> {
        if self.store_ids.contains(&store_id) {
            Ok(())
        } else {
            Err(format!("Unauthorized: no access to store {}", store_id.0))
        }
    }

    /// Check if the user has access to an invoice (via its store).
    async fn authorize_invoice(
        &self,
        invoice_id: &InvoiceId,
    ) -> Result<types::traits::InvoiceData, String> {
        let invoice = InvoiceReader::get(&*self.data_service, invoice_id)
            .await
            .map_err(|e| format!("Database error: {e}"))?
            .ok_or_else(|| format!("Invoice not found: {}", invoice_id.0))?;

        self.authorize_store(invoice.store_id)?;
        Ok(invoice)
    }
}

#[tool_router(server_handler)]
impl EthpayMcpServer {
    /// Create a new invoice for a store. Returns the invoice with payment options
    /// (addresses, amounts, rates) that can be presented to the payer.
    #[tool(
        description = "Create a new invoice for a store. Returns invoice ID, status, and payment options with addresses and amounts."
    )]
    async fn create_invoice(&self, Parameters(args): Parameters<CreateInvoiceArgs>) -> String {
        match self.do_create_invoice(args).await {
            Ok(json) => json,
            Err(e) => format!("{{\"error\": \"{}\"}}", e.replace('"', "\\\"")),
        }
    }

    /// Get full details of an invoice including its payment options.
    #[tool(
        description = "Get an invoice by ID. Returns full invoice data including status, amount, payment options, and expiration."
    )]
    async fn get_invoice(&self, Parameters(args): Parameters<GetInvoiceArgs>) -> String {
        match self.do_get_invoice(args).await {
            Ok(json) => json,
            Err(e) => format!("{{\"error\": \"{}\"}}", e.replace('"', "\\\"")),
        }
    }

    /// List invoices for a store with optional filters.
    #[tool(
        description = "List invoices for a store. Supports filtering by status, currency, and pagination."
    )]
    async fn list_invoices(&self, Parameters(args): Parameters<ListInvoicesArgs>) -> String {
        match self.do_list_invoices(args).await {
            Ok(json) => json,
            Err(e) => format!("{{\"error\": \"{}\"}}", e.replace('"', "\\\"")),
        }
    }

    /// Get all payments received for an invoice.
    #[tool(
        description = "Get payments for an invoice. Returns all payments with chain, asset, amount, tx_hash, and confirmation status."
    )]
    async fn get_invoice_payments(
        &self,
        Parameters(args): Parameters<GetInvoicePaymentsArgs>,
    ) -> String {
        match self.do_get_invoice_payments(args).await {
            Ok(json) => json,
            Err(e) => format!("{{\"error\": \"{}\"}}", e.replace('"', "\\\"")),
        }
    }

    /// Cancel a pending invoice. Only works for invoices in pending, processing,
    /// or partially_paid status.
    #[tool(
        description = "Cancel an invoice. Only works if status is pending, processing, or partially_paid."
    )]
    async fn cancel_invoice(&self, Parameters(args): Parameters<CancelInvoiceArgs>) -> String {
        match self.do_cancel_invoice(args).await {
            Ok(json) => json,
            Err(e) => format!("{{\"error\": \"{}\"}}", e.replace('"', "\\\"")),
        }
    }

    /// Get a simplified payment status for an invoice — useful for polling
    /// whether a payment has been received and confirmed.
    #[tool(
        description = "Get simplified payment status for an invoice: pending/partial/settled/expired/cancelled, with amount_due vs amount_received."
    )]
    async fn get_payment_status(
        &self,
        Parameters(args): Parameters<GetPaymentStatusArgs>,
    ) -> String {
        match self.do_get_payment_status(args).await {
            Ok(json) => json,
            Err(e) => format!("{{\"error\": \"{}\"}}", e.replace('"', "\\\"")),
        }
    }
}

// ---------- Tool implementations ----------

impl EthpayMcpServer {
    async fn do_create_invoice(&self, args: CreateInvoiceArgs) -> Result<String, String> {
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

        let invoice = InvoiceData {
            id: InvoiceId::new(),
            store_id,
            currency: args.currency.clone(),
            status: InvoiceStatus::Pending,
            amount: args.amount.clone(),
            amount_received: "0".to_string(),
            created_at: Utc::now(),
            expires_at,
            metadata: args.metadata,
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
            if let Some(ref monitor) = self.evm_monitor {
                if let Ok(invoice_uuid) = Uuid::parse_str(&invoice.id.0) {
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

    async fn do_get_invoice(&self, args: GetInvoiceArgs) -> Result<String, String> {
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

    async fn do_list_invoices(&self, args: ListInvoicesArgs) -> Result<String, String> {
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

    async fn do_get_invoice_payments(
        &self,
        args: GetInvoicePaymentsArgs,
    ) -> Result<String, String> {
        let id = InvoiceId::from_string(args.invoice_id);
        let _invoice = self.authorize_invoice(&id).await?;

        let payments = PaymentReader::get_for_invoice(&*self.data_service, &id)
            .await
            .map_err(|e| format!("Failed to get payments: {e}"))?;

        let result = serde_json::json!({
            "invoice_id": id.0,
            "payments": payments.iter().map(|p| serde_json::json!({
                "id": p.id.to_string(),
                "chain_id": p.chain_id,
                "asset_symbol": p.asset_symbol,
                "amount": p.amount,
                "tx_hash": p.tx_hash,
                "block_number": p.block_number,
                "from_address": p.from_address,
                "detected_at": p.detected_at.to_rfc3339(),
                "confirmed_at": p.confirmed_at.map(|t| t.to_rfc3339()),
                "reorged": p.reorged,
                "credited_amount": p.credited_amount,
                "rate_used": p.rate_used,
            })).collect::<Vec<_>>(),
        });

        Ok(serde_json::to_string_pretty(&result).unwrap())
    }

    async fn do_cancel_invoice(&self, args: CancelInvoiceArgs) -> Result<String, String> {
        let id = InvoiceId::from_string(args.invoice_id);
        let invoice = self.authorize_invoice(&id).await?;

        match invoice.status {
            InvoiceStatus::Pending | InvoiceStatus::Processing | InvoiceStatus::PartiallyPaid => {}
            other => return Err(format!("Cannot cancel invoice in '{}' status", other)),
        }

        InvoiceWriter::update_status(&*self.data_service, &id, InvoiceStatus::Cancelled)
            .await
            .map_err(|e| format!("Failed to cancel: {e}"))?;

        // Deactivate payment options
        let _ = PaymentOptionWriter::deactivate_for_invoice(&*self.data_service, &id).await;

        Ok(serde_json::json!({
            "id": id.0,
            "status": "cancelled",
            "previous_status": invoice.status.to_string(),
        })
        .to_string())
    }

    async fn do_get_payment_status(&self, args: GetPaymentStatusArgs) -> Result<String, String> {
        let id = InvoiceId::from_string(args.invoice_id);
        let invoice = self.authorize_invoice(&id).await?;

        let payments = PaymentReader::get_for_invoice(&*self.data_service, &id)
            .await
            .map_err(|e| format!("Failed to get payments: {e}"))?;

        let now = Utc::now();
        let confirmed_count = payments.iter().filter(|p| p.confirmed_at.is_some()).count();

        // Compute simplified status
        let simplified_status = match invoice.status {
            InvoiceStatus::Paid => "settled",
            InvoiceStatus::Expired => "expired",
            InvoiceStatus::Cancelled => "cancelled",
            InvoiceStatus::LatePaid => "settled",
            InvoiceStatus::Refunded => "refunded",
            InvoiceStatus::PartiallyPaid => "partial",
            _ if invoice.expires_at < now => "expired",
            _ => "pending",
        };

        let result = serde_json::json!({
            "invoice_id": id.0,
            "status": simplified_status,
            "amount": invoice.amount,
            "amount_received": invoice.amount_received,
            "currency": invoice.currency,
            "expires_at": invoice.expires_at.to_rfc3339(),
            "payment_count": payments.len(),
            "confirmed_count": confirmed_count,
            "is_paid": invoice.status == InvoiceStatus::Paid
                || invoice.status == InvoiceStatus::LatePaid,
            "is_expired": invoice.status == InvoiceStatus::Expired || invoice.expires_at < now,
        });

        Ok(serde_json::to_string_pretty(&result).unwrap())
    }
}

// ---------- Currency conversion helpers ----------
// (Same logic as server/src/api/invoices.rs)

fn convert_to_crypto_smallest_unit(
    amount: &str,
    rate: Decimal,
    decimals: u8,
) -> Result<String, String> {
    let parsed: Decimal = amount.parse().map_err(|_| "Invalid amount".to_string())?;
    if parsed <= Decimal::ZERO {
        return Err("Amount must be positive".into());
    }
    let crypto_amount = parsed
        .checked_mul(rate)
        .ok_or_else(|| "Overflow in rate multiplication".to_string())?;
    let smallest = multiply_by_decimals(crypto_amount, decimals)?;
    decimal_to_integer_string(smallest)
}

fn convert_human_to_smallest_unit(amount: &str, decimals: u8) -> Result<String, String> {
    let parsed: Decimal = amount.parse().map_err(|_| "Invalid amount".to_string())?;
    if parsed <= Decimal::ZERO {
        return Err("Amount must be positive".into());
    }
    let smallest = multiply_by_decimals(parsed, decimals)?;
    decimal_to_integer_string(smallest)
}

fn multiply_by_decimals(value: Decimal, decimals: u8) -> Result<Decimal, String> {
    let ten = Decimal::from(10);
    let mut multiplier = Decimal::ONE;
    for _ in 0..decimals {
        multiplier = multiplier
            .checked_mul(ten)
            .ok_or_else(|| "Overflow computing multiplier".to_string())?;
    }
    value
        .checked_mul(multiplier)
        .ok_or_else(|| "Overflow in smallest units calculation".to_string())
}

fn decimal_to_integer_string(value: Decimal) -> Result<String, String> {
    let floored = value.floor();
    if floored.is_sign_negative() {
        return Err("Negative amount after conversion".into());
    }
    match floored.to_u128() {
        Some(n) => Ok(n.to_string()),
        None => {
            let normalized = floored.normalize();
            if normalized.scale() > 0 {
                return Err("Unexpected decimal places in conversion result".into());
            }
            let mantissa = normalized.mantissa();
            if mantissa < 0 {
                return Err("Negative amount after conversion".into());
            }
            Ok(mantissa.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_to_crypto_basic() {
        let rate = Decimal::from_str_exact("0.0005").unwrap();
        let result = convert_to_crypto_smallest_unit("100", rate, 18).unwrap();
        assert_eq!(result, "50000000000000000");
    }

    #[test]
    fn test_convert_to_crypto_usdc() {
        let rate = Decimal::ONE;
        let result = convert_to_crypto_smallest_unit("100", rate, 6).unwrap();
        assert_eq!(result, "100000000");
    }

    #[test]
    fn test_convert_human_to_smallest_eth() {
        let result = convert_human_to_smallest_unit("1.5", 18).unwrap();
        assert_eq!(result, "1500000000000000000");
    }

    #[test]
    fn test_convert_rejects_zero() {
        let rate = Decimal::from_str_exact("0.0005").unwrap();
        assert!(convert_to_crypto_smallest_unit("0", rate, 18).is_err());
    }

    #[test]
    fn test_convert_rejects_negative() {
        let rate = Decimal::from_str_exact("0.0005").unwrap();
        assert!(convert_to_crypto_smallest_unit("-100", rate, 18).is_err());
    }

    #[test]
    fn test_convert_rejects_invalid() {
        let rate = Decimal::from_str_exact("0.0005").unwrap();
        assert!(convert_to_crypto_smallest_unit("abc", rate, 18).is_err());
    }

    #[test]
    fn test_convert_human_rejects_negative() {
        assert!(convert_human_to_smallest_unit("-1", 18).is_err());
    }

    #[test]
    fn test_convert_floors_result() {
        let rate = Decimal::from_str_exact("0.0005").unwrap();
        let result = convert_to_crypto_smallest_unit("1.001", rate, 18).unwrap();
        assert_eq!(result, "500500000000000");
    }
}
