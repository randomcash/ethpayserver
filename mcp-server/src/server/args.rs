//! Tool parameter types for the MCP server.

use rmcp::schemars;

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
    #[schemars(description = "Optional customer email for payment receipt")]
    pub customer_email: Option<String>,
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
