//! MCP server handler with tool implementations.

mod args;
mod convert;
mod invoice;
mod payment;

use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};

use auth::UserId;
use data_service::PgDataService;
use evm::monitor::bridge::{COMMANDS_CHANNEL, EVENTS_CHANNEL, RedisBridge};
use rates::RateProvider;
use types::{InvoiceId, InvoiceReader, StoreId};

pub use args::{
    CancelInvoiceArgs, CreateInvoiceArgs, GetInvoiceArgs, GetInvoicePaymentsArgs,
    GetPaymentStatusArgs, ListInvoicesArgs,
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

// ---------- Server ----------

#[derive(Clone)]
pub struct EthpayMcpServer {
    data_service: Arc<PgDataService>,
    #[allow(dead_code)]
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
