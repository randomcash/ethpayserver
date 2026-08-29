//! Payment-related API types.

use serde::{Deserialize, Serialize};

/// Payment data from the API.
///
/// Mirrors `PaymentResponse` from the backend.
/// A payment is an actual received transaction belonging to an invoice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payment {
    pub id: String,
    /// Store the payment's invoice belongs to (list endpoints only).
    #[serde(default)]
    pub store_id: Option<String>,
    /// Store name, populated by the list endpoint so an "All Stores" view can
    /// label each row (RCS-171).
    #[serde(default)]
    pub store_name: Option<String>,
    /// Chain ID (EIP-155).
    pub chain_id: u64,
    /// Invoice ID this payment belongs to.
    pub invoice_id: String,
    /// Transaction hash.
    pub tx_hash: String,
    /// Amount received (smallest unit as string).
    pub amount: String,
    /// Asset symbol (e.g., "ETH", "USDC").
    pub asset_symbol: String,
    /// Token contract address (for ERC20 tokens).
    pub token_address: Option<String>,
    /// Block number where the payment was included.
    pub block_number: Option<u64>,
    /// Sender address (if known).
    pub from_address: Option<String>,
    /// When the payment was detected (ISO 8601 string).
    pub detected_at: String,
    /// When the payment reached required confirmations (None = awaiting).
    pub confirmed_at: Option<String>,
    /// Whether this payment was invalidated by a chain reorganization.
    #[serde(default)]
    pub reorged: bool,
    /// Token decimals (for display formatting).
    #[serde(default = "default_decimals")]
    pub decimals: u8,
}

fn default_decimals() -> u8 {
    18
}

/// Payment option for an invoice (a specific chain/asset the payer can use).
///
/// Mirrors `PaymentOptionResponse` from the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentOption {
    pub id: String,
    /// Format: "{ASSET}-{CHAIN_ID}" e.g., "ETH-1", "USDC-137".
    pub payment_method_id: String,
    /// EIP-155 chain ID.
    pub chain_id: u64,
    /// Asset symbol (e.g., "ETH", "USDC").
    pub asset_symbol: String,
    /// Token contract address (None for native assets).
    pub token_address: Option<String>,
    /// Token decimals.
    pub decimals: u8,
    /// Derived wallet address for this payment.
    pub payment_address: String,
    /// Amount due in smallest units.
    pub amount: String,
    /// Exchange rate used (if currency conversion involved).
    pub rate: Option<String>,
    /// Whether this payment option is still active.
    pub is_active: bool,
}

/// Payment list response from the backend.
///
/// Mirrors `PaymentListResponse` from the server API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentListResponse {
    pub total: i64,
    pub payments: Vec<Payment>,
}

/// Response from tx hash lookup endpoint.
///
/// Mirrors `TxHashLookupResponse` from the server API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxHashLookupResponse {
    pub invoice: super::invoice::Invoice,
    pub payment: Payment,
}
