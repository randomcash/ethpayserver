//! API types for ethpayserver.
//!
//! These types mirror the backend types from payserver-commons/types.

use serde::{Deserialize, Serialize};

/// Status of an invoice.
///
/// Mirrors `types::InvoiceStatus` from the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvoiceStatus {
    /// Invoice created, awaiting payment.
    Pending,
    /// Payment detected but not confirmed.
    Processing,
    /// Payment partially received.
    PartiallyPaid,
    /// Payment fully received and confirmed.
    Paid,
    /// Invoice expired without payment.
    Expired,
    /// Invoice cancelled.
    Cancelled,
    /// Payment refunded.
    Refunded,
    /// Payment received after invoice expired.
    LatePaid,
}

impl InvoiceStatus {
    /// Returns the display label for this status.
    pub fn label(&self) -> &'static str {
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

    /// Returns the CSS class for styling this status.
    pub fn css_class(&self) -> &'static str {
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

    /// Returns true if this is a final status (no more changes expected).
    pub fn is_final(&self) -> bool {
        matches!(
            self,
            InvoiceStatus::Paid
                | InvoiceStatus::Expired
                | InvoiceStatus::Cancelled
                | InvoiceStatus::Refunded
                | InvoiceStatus::LatePaid
        )
    }
}

impl Default for InvoiceStatus {
    fn default() -> Self {
        InvoiceStatus::Pending
    }
}

/// Asset type for payments.
///
/// Mirrors `types::AssetType` from the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AssetType {
    /// Native network currency (ETH, BTC, POL, etc.)
    #[default]
    Native,
    /// ERC20 token (for EVM networks)
    ERC20,
}

/// Invoice data from the API.
///
/// Mirrors `types::InvoiceData` from the backend.
/// An invoice is network-agnostic: it represents a payment request in a
/// specific currency (which can be fiat like "USD" or crypto like "ETH").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invoice {
    pub id: String,
    /// The store this invoice belongs to.
    pub store_id: String,
    /// Invoice currency (e.g., "USD", "EUR", "ETH").
    pub currency: String,
    /// Invoice status.
    #[serde(default)]
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
}

/// Payment data from the API.
///
/// Mirrors `types::PaymentData` from the backend.
/// A payment is an actual received transaction belonging to an invoice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payment {
    pub id: String,
    pub invoice_id: String,
    /// EIP-155 chain ID where this payment was received.
    pub chain_id: u64,
    /// Asset type (native or ERC20).
    #[serde(default)]
    pub asset_type: AssetType,
    /// Amount received (smallest unit as string).
    pub amount: String,
    /// Asset symbol (e.g., "ETH", "USDC").
    pub asset_symbol: String,
    /// Token contract address (for ERC20 tokens).
    pub token_address: Option<String>,
    /// Transaction hash.
    pub tx_hash: String,
    /// Block number where the payment was included.
    pub block_number: Option<u64>,
    /// When the payment was detected (ISO 8601 string).
    pub detected_at: String,
    /// When the payment reached required confirmations (None = awaiting).
    pub confirmed_at: Option<String>,
    /// Sender address (if known).
    pub from_address: Option<String>,
    /// Whether this payment was invalidated by a chain reorganization.
    #[serde(default)]
    pub reorged: bool,
    /// The payment's value credited toward the invoice total, in invoice currency.
    pub credited_amount: Option<String>,
    /// Exchange rate used to calculate credited_amount.
    pub rate_used: Option<String>,
}

/// Store data from the API.
///
/// Mirrors `Store` / `StoreInfo` from the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Store {
    pub id: String,
    pub name: String,
    pub website: Option<String>,
    /// Whether the store is archived (soft-deleted).
    #[serde(default)]
    pub archived: bool,
    pub created_at: String,
}

/// Store payment method - defines which chains/tokens a store accepts.
///
/// Mirrors `StorePaymentMethod` from the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorePaymentMethod {
    pub id: String,
    pub store_id: String,
    /// EIP-155 chain ID (1 = Ethereum, 137 = Polygon, etc.)
    pub chain_id: u64,
    /// Token contract address for ERC20, None for native asset.
    pub token_address: Option<String>,
    /// Asset symbol (ETH, USDC, USDT, etc.)
    pub asset_symbol: String,
    /// Token decimals (18 for ETH, 6 for USDC/USDT).
    pub decimals: u8,
    /// BIP-32 extended public key for HD wallet derivation.
    pub xpub: String,
    /// Next derivation index to use.
    pub derivation_index: i32,
    /// Whether this payment method is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub created_at: String,
}

fn default_true() -> bool {
    true
}

/// Store webhook configuration.
///
/// Mirrors `StoreWebhook` from the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreWebhook {
    pub id: String,
    pub store_id: String,
    /// Webhook endpoint URL.
    pub webhook_url: String,
    /// HMAC-SHA256 secret for payload signing (masked in responses).
    pub webhook_secret: String,
    /// Whether webhooks are enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Store role for user permissions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreRole {
    pub id: String,
    pub store_id: Option<String>,
    pub role: String,
    pub permissions: Vec<String>,
}

/// User's relationship to a store with role info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserStoreInfo {
    pub store: Store,
    pub role: StoreRole,
}

/// Wallet data from the API (legacy, kept for compatibility).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wallet {
    pub id: String,
    pub name: String,
    pub address: String,
    pub derivation_path: String,
    pub enabled_chains: Vec<u64>,
    pub created_at: String,
}

/// Dashboard statistics.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DashboardStats {
    pub total_invoices: u64,
    pub pending_invoices: u64,
    pub paid_invoices: u64,
    pub expired_invoices: u64,
    pub total_payments: u64,
    pub total_volume_usd: String,
}

/// Create invoice request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInvoiceRequest {
    pub store_id: String,
    pub amount: String,
    pub currency: String,
    pub chain_id: Option<u64>,
    pub metadata: Option<serde_json::Value>,
}

/// Create store request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateStoreRequest {
    pub name: String,
    pub website: Option<String>,
}

/// Update store request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateStoreRequest {
    pub name: Option<String>,
    pub website: Option<String>,
}

/// Create payment method request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePaymentMethodRequest {
    pub chain_id: u64,
    pub token_address: Option<String>,
    pub asset_symbol: String,
    pub decimals: u8,
    pub xpub: String,
}

/// Update webhook request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateWebhookRequest {
    pub webhook_url: String,
    pub enabled: bool,
}

/// Paginated response wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invoice_status() {
        assert_eq!(InvoiceStatus::Pending.label(), "Pending");
        assert_eq!(InvoiceStatus::Paid.css_class(), "badge badge-success");
        assert!(InvoiceStatus::Paid.is_final());
        assert!(!InvoiceStatus::Pending.is_final());
    }

    #[test]
    fn test_invoice_serialization() {
        let invoice = Invoice {
            id: "inv_001".to_string(),
            store_id: "store_001".to_string(),
            amount: "100.00".to_string(),
            currency: "USD".to_string(),
            status: InvoiceStatus::Pending,
            amount_received: "0.00".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            expires_at: "2024-01-02T00:00:00Z".to_string(),
            metadata: None,
        };

        let json = serde_json::to_string(&invoice).unwrap();
        let parsed: Invoice = serde_json::from_str(&json).unwrap();

        assert_eq!(invoice.id, parsed.id);
        assert_eq!(invoice.status, parsed.status);
    }

    #[test]
    fn test_payment_serialization() {
        let payment = Payment {
            id: "pay_001".to_string(),
            invoice_id: "inv_001".to_string(),
            chain_id: 1,
            asset_type: AssetType::Native,
            amount: "50000000000000000".to_string(),
            asset_symbol: "ETH".to_string(),
            token_address: None,
            tx_hash: "0xabc123...".to_string(),
            block_number: Some(19000000),
            detected_at: "2024-01-01T00:00:00Z".to_string(),
            confirmed_at: Some("2024-01-01T00:05:00Z".to_string()),
            from_address: Some("0x1234...".to_string()),
            reorged: false,
            credited_amount: Some("100.00".to_string()),
            rate_used: Some("0.0005".to_string()),
        };

        let json = serde_json::to_string(&payment).unwrap();
        let parsed: Payment = serde_json::from_str(&json).unwrap();

        assert_eq!(payment.id, parsed.id);
        assert_eq!(payment.chain_id, parsed.chain_id);
        assert_eq!(payment.confirmed_at, parsed.confirmed_at);
    }

    #[test]
    fn test_store_serialization() {
        let store = Store {
            id: "store_001".to_string(),
            name: "Test Store".to_string(),
            website: Some("https://example.com".to_string()),
            archived: false,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&store).unwrap();
        let parsed: Store = serde_json::from_str(&json).unwrap();

        assert_eq!(store.id, parsed.id);
        assert_eq!(store.name, parsed.name);
    }

    #[test]
    fn test_wallet_serialization() {
        let wallet = Wallet {
            id: "wallet_001".to_string(),
            name: "Main Wallet".to_string(),
            address: "0x1234...".to_string(),
            derivation_path: "m/44'/60'/0'/0/0".to_string(),
            enabled_chains: vec![1, 137, 42161],
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&wallet).unwrap();
        let parsed: Wallet = serde_json::from_str(&json).unwrap();

        assert_eq!(wallet.id, parsed.id);
        assert_eq!(wallet.derivation_path, parsed.derivation_path);
    }

    #[test]
    fn test_dashboard_stats_default() {
        let stats = DashboardStats::default();

        assert_eq!(stats.total_invoices, 0);
        assert_eq!(stats.pending_invoices, 0);
        assert_eq!(stats.total_payments, 0);
    }
}
