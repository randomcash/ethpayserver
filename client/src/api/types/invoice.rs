//! Invoice-related API types.

use serde::{Deserialize, Serialize};

// Re-export InvoiceStatus from the shared types crate.
pub use types::InvoiceStatus;

use super::payment::{Payment, PaymentOption};

/// UI-specific display methods for InvoiceStatus.
pub trait InvoiceStatusExt {
    fn label(&self) -> &'static str;
    fn css_class(&self) -> &'static str;
}

impl InvoiceStatusExt for InvoiceStatus {
    fn label(&self) -> &'static str {
        match self {
            InvoiceStatus::Pending => "Pending",
            InvoiceStatus::Processing => "Processing",
            InvoiceStatus::PartiallyPaid => "Partially Paid",
            InvoiceStatus::Paid => "Paid",
            InvoiceStatus::Expired => "Expired",
            InvoiceStatus::Cancelled => "Cancelled",
            InvoiceStatus::Refunded => "Refunded",
            InvoiceStatus::LatePaid => "Late Paid",
        }
    }

    fn css_class(&self) -> &'static str {
        match self {
            InvoiceStatus::Pending => "badge badge-warning",
            InvoiceStatus::Processing => "badge badge-info",
            InvoiceStatus::PartiallyPaid => "badge badge-warning",
            InvoiceStatus::Paid => "badge badge-success",
            InvoiceStatus::Expired => "badge badge-error",
            InvoiceStatus::Cancelled => "badge badge-neutral",
            InvoiceStatus::Refunded => "badge badge-neutral",
            InvoiceStatus::LatePaid => "badge badge-info",
        }
    }
}

fn default_invoice_status() -> InvoiceStatus {
    InvoiceStatus::Pending
}

/// Invoice data from the API.
///
/// Mirrors `InvoiceResponse` from the backend.
/// An invoice is network-agnostic: it represents a payment request in a
/// specific currency (which can be fiat like "USD" or crypto like "ETH").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invoice {
    pub id: String,
    /// Invoice currency (e.g., "USD", "EUR", "ETH").
    pub currency: String,
    /// Invoice status.
    #[serde(default = "default_invoice_status")]
    pub status: InvoiceStatus,
    /// Amount requested in the invoice currency.
    pub amount: String,
    /// Total amount received across all payments, converted to invoice currency.
    #[serde(default)]
    pub amount_received: String,
    /// When the invoice was created (ISO 8601 string).
    pub created_at: String,
    /// When the invoice expires (ISO 8601 string).
    pub expires_at: String,
    /// Optional metadata.
    pub metadata: Option<serde_json::Value>,
    /// Available payment options for this invoice.
    #[serde(default)]
    pub payment_options: Vec<PaymentOption>,
}

/// Invoice status response (comprehensive view with payments).
///
/// Mirrors `InvoiceStatusResponse` from the backend.
/// Returned by `GET /invoices/{id}/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceStatusResponse {
    pub id: String,
    pub status: InvoiceStatus,
    pub amount: String,
    pub amount_received: String,
    pub currency: String,
    pub expires_at: String,
    pub payment_count: usize,
    pub confirmed_count: usize,
    pub is_paid: bool,
    pub is_expired: bool,
    pub payment_options: Vec<PaymentOption>,
    pub payments: Vec<Payment>,
}

/// Invoice list response from the backend.
///
/// Mirrors `InvoiceListResponse` from the server API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceListResponse {
    pub total: i64,
    pub invoices: Vec<Invoice>,
}

/// Create invoice request.
///
/// Mirrors `CreateInvoiceRequest` from the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInvoiceRequest {
    pub store_id: String,
    pub currency: String,
    pub amount: String,
    /// Invoice expiration in seconds (default: 900 = 15 minutes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiration_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer_email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_url: Option<String>,
}

/// Privacy-filtered payment view for the public checkout page.
///
/// Mirrors `CheckoutPaymentInfo` from the server. Distinct from `Payment`
/// because the checkout endpoint deliberately omits `from_address` (sender
/// wallet) and `reorged` (internal state) — anyone with the invoice link
/// can hit the endpoint, so sender addresses must not leak.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckoutPaymentInfo {
    pub id: String,
    pub chain_id: u64,
    pub tx_hash: String,
    pub amount: String,
    pub asset_symbol: String,
    pub token_address: Option<String>,
    pub block_number: Option<u64>,
    pub detected_at: String,
    pub confirmed_at: Option<String>,
}

/// Public checkout response.
///
/// Mirrors `CheckoutResponse` from the server checkout API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckoutResponse {
    pub id: String,
    pub currency: String,
    pub status: String,
    pub amount: String,
    pub amount_received: String,
    pub expires_at: String,
    pub is_expired: bool,
    pub is_paid: bool,
    pub payment_options: Vec<PaymentOption>,
    pub payments: Vec<CheckoutPaymentInfo>,
}
