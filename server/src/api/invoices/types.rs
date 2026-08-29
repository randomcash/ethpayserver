use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use ::types::{PaymentOptionData, traits::PaymentData};

/// Request to create a new invoice.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateInvoiceRequest {
    /// Store ID this invoice belongs to.
    pub store_id: Uuid,
    /// Invoice currency (e.g., "USD", "ETH", "BTC").
    /// For asset-denominated invoices (testing), use the asset symbol directly.
    pub currency: String,
    /// Amount in the currency's standard unit (e.g., "100.00" for USD, "0.1" for ETH).
    /// For asset-denominated invoices, this is in the asset's smallest unit (wei, satoshi).
    pub amount: String,
    /// Expiration in seconds from now (default: 900 = 15 minutes).
    pub expiration_seconds: Option<u64>,
    /// Optional metadata.
    pub metadata: Option<serde_json::Value>,
    /// Optional customer email for payment receipt.
    pub customer_email: Option<String>,
    /// Optional webhook URL.
    pub webhook_url: Option<String>,
    /// Optional redirect URL after payment.
    pub redirect_url: Option<String>,
}

/// Payment option response for an invoice.
#[derive(Debug, Serialize, ToSchema)]
pub struct PaymentOptionResponse {
    /// Payment option ID.
    pub id: String,
    /// Payment method ID (e.g., "ETH-1", "USDC-137").
    pub payment_method_id: String,
    /// Chain ID (EIP-155).
    pub chain_id: u64,
    /// Asset symbol.
    pub asset_symbol: String,
    /// Token contract address (for ERC20, null for native).
    pub token_address: Option<String>,
    /// Asset decimals.
    pub decimals: u8,
    /// Payment address.
    pub payment_address: String,
    /// Amount in the asset's smallest unit.
    pub amount: String,
    /// Exchange rate at time of creation.
    pub rate: Option<String>,
    /// Whether this option is active.
    pub is_active: bool,
}

impl From<PaymentOptionData> for PaymentOptionResponse {
    fn from(po: PaymentOptionData) -> Self {
        Self {
            id: po.id.0.to_string(),
            payment_method_id: po.payment_method_id.0,
            chain_id: po.chain_id,
            asset_symbol: po.asset_symbol,
            token_address: po.token_address,
            decimals: po.decimals,
            payment_address: po.payment_address,
            amount: po.amount,
            rate: po.rate,
            is_active: po.is_active,
        }
    }
}

/// Invoice response (network-agnostic).
#[derive(Debug, Serialize, ToSchema)]
pub struct InvoiceResponse {
    /// Invoice ID.
    pub id: String,
    /// Store this invoice belongs to.
    pub store_id: String,
    /// Store name, when the caller needs to tell stores apart (RCS-171).
    ///
    /// Only the list endpoints resolve this: a single-invoice response is
    /// always read in a context that already knows the store, and looking the
    /// name up there would be a query per request for a field nobody renders.
    pub store_name: Option<String>,
    /// Invoice currency (e.g., "USD", "EUR", "ETH").
    pub currency: String,
    /// Status.
    pub status: String,
    /// Requested amount in the invoice currency.
    pub amount: String,
    /// Amount received so far (in invoice currency terms).
    pub amount_received: String,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Expiration timestamp.
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// Metadata.
    pub metadata: Option<serde_json::Value>,
    /// Customer email for payment receipt (if set).
    pub customer_email: Option<String>,
    /// Payment options for this invoice.
    pub payment_options: Vec<PaymentOptionResponse>,
}

/// Payment response.
#[derive(Debug, Serialize, ToSchema)]
pub struct PaymentResponse {
    /// Payment ID.
    pub id: String,
    /// Store the payment's invoice belongs to (list endpoints only).
    pub store_id: Option<String>,
    /// Store name (list endpoints only) — see [`InvoiceResponse::store_name`].
    pub store_name: Option<String>,
    /// Chain ID (EIP-155).
    pub chain_id: u64,
    /// Invoice ID this payment belongs to.
    pub invoice_id: String,
    /// Transaction hash.
    pub tx_hash: String,
    /// Amount received.
    pub amount: String,
    /// Asset symbol.
    pub asset_symbol: String,
    /// Token contract address (for ERC20).
    pub token_address: Option<String>,
    /// Block number.
    pub block_number: Option<u64>,
    /// Sender address.
    pub from_address: Option<String>,
    /// When the payment was detected.
    pub detected_at: chrono::DateTime<chrono::Utc>,
    /// When the payment was confirmed (None = awaiting confirmation).
    pub confirmed_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether this payment was invalidated by a chain reorg.
    pub reorged: bool,
    /// Token decimals (for display formatting).
    pub decimals: u8,
}

impl From<PaymentData> for PaymentResponse {
    fn from(p: PaymentData) -> Self {
        let decimals = token_decimals(&p.asset_symbol, p.token_address.as_deref());
        Self {
            id: p.id.to_string(),
            // Payments carry no store of their own; only the list endpoint,
            // which already resolves the invoice, fills these in.
            store_id: None,
            store_name: None,
            chain_id: p.chain_id,
            invoice_id: p.invoice_id.0,
            tx_hash: p.tx_hash,
            amount: p.amount,
            asset_symbol: p.asset_symbol,
            token_address: p.token_address,
            block_number: p.block_number,
            from_address: p.from_address,
            detected_at: p.detected_at,
            confirmed_at: p.confirmed_at,
            reorged: p.reorged,
            decimals,
        }
    }
}

/// Resolve token decimals from symbol and optional contract address.
fn token_decimals(symbol: &str, token_address: Option<&str>) -> u8 {
    match symbol {
        "ETH" | "POL" | "MATIC" | "FTM" | "xDAI" | "DAI" | "WETH" => 18,
        "USDC" | "USDT" => 6,
        "WBTC" => 8,
        _ => {
            // ERC20 without a known symbol — check well-known contract addresses.
            if let Some(addr) = token_address {
                let addr_lower = addr.to_lowercase();
                // USDC on major chains
                if addr_lower == "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
                    || addr_lower == "0x2791bca1f2de4661ed88a30c99a7a9449aa84174"
                    || addr_lower == "0x3c499c542cef5e3811e1192ce70d8cc03d5c3359"
                {
                    return 6;
                }
                // WBTC on Ethereum
                if addr_lower == "0x2260fac5e5542a773aa44fbcfedf7c193bc2c599" {
                    return 8;
                }
            }
            18 // default to 18 for unknown tokens
        }
    }
}

/// Paginated payment list response.
#[derive(Debug, Serialize, ToSchema)]
pub struct PaymentListResponse {
    /// Total number of matching payments.
    pub total: i64,
    /// Payments in this page.
    pub payments: Vec<PaymentResponse>,
}

/// Response for tx hash lookup — returns the matching invoice and payment.
#[derive(Debug, Serialize, ToSchema)]
pub struct TxHashLookupResponse {
    /// The invoice linked to this transaction.
    pub invoice: InvoiceResponse,
    /// The payment matching the tx hash.
    pub payment: PaymentResponse,
}

/// Query parameters for listing payments.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListPaymentsQuery {
    /// Filter by store ID.
    pub store_id: Option<Uuid>,
    /// Filter by status (confirmed, pending).
    pub status: Option<String>,
    /// Maximum number of results.
    pub limit: Option<i64>,
    /// Offset for pagination.
    pub offset: Option<i64>,
}

/// Invoice status response with payment details.
#[derive(Debug, Serialize, ToSchema)]
pub struct InvoiceStatusResponse {
    /// Invoice ID.
    pub id: String,
    /// Current status.
    pub status: String,
    /// Requested amount in invoice currency.
    pub amount: String,
    /// Amount received so far.
    pub amount_received: String,
    /// Invoice currency.
    pub currency: String,
    /// Expiration timestamp.
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// Number of payments received.
    pub payment_count: usize,
    /// Number of confirmed payments.
    pub confirmed_count: usize,
    /// Whether the invoice is fully paid.
    pub is_paid: bool,
    /// Whether the invoice is expired.
    pub is_expired: bool,
    /// Payment options for this invoice.
    pub payment_options: Vec<PaymentOptionResponse>,
    /// Payments received for this invoice.
    pub payments: Vec<PaymentResponse>,
}

/// Query parameters for listing invoices.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListInvoicesQuery {
    /// Filter by store ID.
    pub store_id: Option<Uuid>,
    /// Filter by status.
    pub status: Option<String>,
    /// Filter by currency.
    pub currency: Option<String>,
    /// Maximum number of results.
    pub limit: Option<i64>,
    /// Offset for pagination.
    pub offset: Option<i64>,
}

/// Paginated invoice list response.
#[derive(Debug, Serialize, ToSchema)]
pub struct InvoiceListResponse {
    /// Total number of matching invoices.
    pub total: i64,
    /// Invoices in this page.
    pub invoices: Vec<InvoiceResponse>,
}
