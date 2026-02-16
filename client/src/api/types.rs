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
/// Mirrors `PaymentMethodResponse` from the backend API.
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
    /// BIP-32 extended public key (masked for security).
    pub xpub_masked: String,
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
/// Mirrors `WebhookResponse` from the backend API.
/// `webhook_secret` is only present in the response after configure (PUT),
/// not on GET (backend hides it for security).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreWebhook {
    pub id: String,
    pub store_id: String,
    /// Webhook endpoint URL.
    pub webhook_url: String,
    /// HMAC-SHA256 secret for payload signing. Only returned on configure (PUT).
    pub webhook_secret: Option<String>,
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

/// Update payment method request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePaymentMethodRequest {
    pub enabled: Option<bool>,
    pub xpub: Option<String>,
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

    // =========================================================================
    // Store types
    // =========================================================================

    #[test]
    fn test_store_deserialization_from_backend() {
        // Simulates the JSON the backend actually sends (StoreResponse)
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "name": "My Shop",
            "website": "https://myshop.com",
            "owner_id": "660e8400-e29b-41d4-a716-446655440000",
            "archived": false,
            "created_at": "2024-06-15T10:30:00Z"
        }"#;
        let store: Store = serde_json::from_str(json).unwrap();
        assert_eq!(store.id, "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(store.name, "My Shop");
        assert_eq!(store.website, Some("https://myshop.com".to_string()));
        assert!(!store.archived);
        // owner_id is ignored by client-side Store (extra fields tolerated by serde default)
    }

    #[test]
    fn test_store_deserialization_minimal() {
        // Backend may omit optional fields
        let json = r#"{
            "id": "abc",
            "name": "Bare Store",
            "website": null,
            "created_at": "2024-01-01T00:00:00Z"
        }"#;
        let store: Store = serde_json::from_str(json).unwrap();
        assert_eq!(store.name, "Bare Store");
        assert!(store.website.is_none());
        assert!(!store.archived); // default
    }

    #[test]
    fn test_create_store_request() {
        let req = CreateStoreRequest {
            name: "New Store".to_string(),
            website: Some("https://new.store".to_string()),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["name"], "New Store");
        assert_eq!(json["website"], "https://new.store");
    }

    #[test]
    fn test_create_store_request_without_website() {
        let req = CreateStoreRequest {
            name: "Simple".to_string(),
            website: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["name"], "Simple");
        assert!(json["website"].is_null());
    }

    #[test]
    fn test_update_store_request_serialization() {
        let req = UpdateStoreRequest {
            name: Some("Updated Name".to_string()),
            website: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["name"], "Updated Name");
        assert!(json["website"].is_null());
    }

    // =========================================================================
    // InvoiceStatus coverage
    // =========================================================================

    #[test]
    fn test_invoice_status_all_labels() {
        assert_eq!(InvoiceStatus::Processing.label(), "Processing");
        assert_eq!(InvoiceStatus::PartiallyPaid.label(), "Partially Paid");
        assert_eq!(InvoiceStatus::Expired.label(), "Expired");
        assert_eq!(InvoiceStatus::Cancelled.label(), "Cancelled");
        assert_eq!(InvoiceStatus::Refunded.label(), "Refunded");
        assert_eq!(InvoiceStatus::LatePaid.label(), "Late Paid");
    }

    #[test]
    fn test_invoice_status_all_css_classes() {
        assert_eq!(InvoiceStatus::Pending.css_class(), "badge badge-warning");
        assert_eq!(InvoiceStatus::Processing.css_class(), "badge badge-info");
        assert_eq!(InvoiceStatus::PartiallyPaid.css_class(), "badge badge-warning");
        assert_eq!(InvoiceStatus::Expired.css_class(), "badge badge-error");
        assert_eq!(InvoiceStatus::Cancelled.css_class(), "badge badge-neutral");
        assert_eq!(InvoiceStatus::Refunded.css_class(), "badge badge-neutral");
        assert_eq!(InvoiceStatus::LatePaid.css_class(), "badge badge-info");
    }

    #[test]
    fn test_invoice_status_is_final() {
        // Final statuses
        assert!(InvoiceStatus::Paid.is_final());
        assert!(InvoiceStatus::Expired.is_final());
        assert!(InvoiceStatus::Cancelled.is_final());
        assert!(InvoiceStatus::Refunded.is_final());
        assert!(InvoiceStatus::LatePaid.is_final());
        // Non-final
        assert!(!InvoiceStatus::Pending.is_final());
        assert!(!InvoiceStatus::Processing.is_final());
        assert!(!InvoiceStatus::PartiallyPaid.is_final());
    }

    #[test]
    fn test_invoice_status_default() {
        assert_eq!(InvoiceStatus::default(), InvoiceStatus::Pending);
    }

    #[test]
    fn test_invoice_status_serde_roundtrip() {
        // snake_case serialization
        let json = serde_json::to_string(&InvoiceStatus::PartiallyPaid).unwrap();
        assert_eq!(json, "\"partially_paid\"");
        let parsed: InvoiceStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, InvoiceStatus::PartiallyPaid);
    }

    // =========================================================================
    // AssetType
    // =========================================================================

    #[test]
    fn test_asset_type_default() {
        assert_eq!(AssetType::default(), AssetType::Native);
    }

    #[test]
    fn test_asset_type_serde_roundtrip() {
        let json = serde_json::to_string(&AssetType::ERC20).unwrap();
        assert_eq!(json, "\"e_r_c20\""); // rename_all = snake_case on ERC20
        let parsed: AssetType = serde_json::from_str("\"e_r_c20\"").unwrap();
        assert_eq!(parsed, AssetType::ERC20);

        let json = serde_json::to_string(&AssetType::Native).unwrap();
        assert_eq!(json, "\"native\"");
        let parsed: AssetType = serde_json::from_str("\"native\"").unwrap();
        assert_eq!(parsed, AssetType::Native);
    }

    // =========================================================================
    // Payment method & webhook
    // =========================================================================

    #[test]
    fn test_store_payment_method_serialization() {
        let pm = StorePaymentMethod {
            id: "pm_001".to_string(),
            store_id: "store_001".to_string(),
            chain_id: 1,
            token_address: None,
            asset_symbol: "ETH".to_string(),
            xpub_masked: "xpub12...pub123".to_string(),
            derivation_index: 0,
            enabled: true,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&pm).unwrap();
        let parsed: StorePaymentMethod = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.chain_id, 1);
        assert_eq!(parsed.asset_symbol, "ETH");
        assert!(parsed.enabled);
        assert!(parsed.token_address.is_none());
    }

    #[test]
    fn test_store_payment_method_erc20() {
        let pm = StorePaymentMethod {
            id: "pm_002".to_string(),
            store_id: "store_001".to_string(),
            chain_id: 137,
            token_address: Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string()),
            asset_symbol: "USDC".to_string(),
            xpub_masked: "xpub45...pub456".to_string(),
            derivation_index: 5,
            enabled: false,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&pm).unwrap();
        let parsed: StorePaymentMethod = serde_json::from_str(&json).unwrap();
        assert!(!parsed.enabled);
        assert!(parsed.token_address.is_some());
    }

    #[test]
    fn test_store_payment_method_enabled_default() {
        // When 'enabled' is missing from JSON, it should default to true
        let json = r#"{
            "id": "pm_003",
            "store_id": "s1",
            "chain_id": 1,
            "token_address": null,
            "asset_symbol": "ETH",
            "xpub_masked": "xpub...pub",
            "derivation_index": 0,
            "created_at": "2024-01-01T00:00:00Z"
        }"#;
        let pm: StorePaymentMethod = serde_json::from_str(json).unwrap();
        assert!(pm.enabled);
    }

    #[test]
    fn test_store_payment_method_from_backend_json() {
        // Simulates the actual PaymentMethodResponse JSON from the backend
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "store_id": "660e8400-e29b-41d4-a716-446655440000",
            "chain_id": 11155111,
            "token_address": null,
            "asset_symbol": "ETH",
            "xpub_masked": "xpub6CUG...Ht4QRnxv",
            "derivation_index": 3,
            "enabled": true,
            "created_at": "2024-06-15T10:30:00Z"
        }"#;
        let pm: StorePaymentMethod = serde_json::from_str(json).unwrap();
        assert_eq!(pm.id, "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(pm.chain_id, 11155111);
        assert_eq!(pm.asset_symbol, "ETH");
        assert_eq!(pm.xpub_masked, "xpub6CUG...Ht4QRnxv");
        assert_eq!(pm.derivation_index, 3);
        assert!(pm.enabled);
        assert!(pm.token_address.is_none());
    }

    #[test]
    fn test_store_payment_method_from_backend_erc20_json() {
        // Backend response for an ERC20 payment method
        let json = r#"{
            "id": "770e8400-e29b-41d4-a716-446655440000",
            "store_id": "660e8400-e29b-41d4-a716-446655440000",
            "chain_id": 1,
            "token_address": "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
            "asset_symbol": "USDC",
            "xpub_masked": "xpub6D4B...9kW3F2Rq",
            "derivation_index": 0,
            "enabled": false,
            "created_at": "2024-06-15T10:30:00Z"
        }"#;
        let pm: StorePaymentMethod = serde_json::from_str(json).unwrap();
        assert_eq!(pm.chain_id, 1);
        assert_eq!(pm.asset_symbol, "USDC");
        assert_eq!(pm.token_address.as_deref(), Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"));
        assert!(!pm.enabled);
    }

    #[test]
    fn test_create_payment_method_request() {
        let req = CreatePaymentMethodRequest {
            chain_id: 1,
            token_address: None,
            asset_symbol: "ETH".to_string(),
            decimals: 18,
            xpub: "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz".to_string(),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["chain_id"], 1);
        assert!(json["token_address"].is_null());
        assert_eq!(json["asset_symbol"], "ETH");
        assert_eq!(json["decimals"], 18);
        assert!(json["xpub"].as_str().unwrap().starts_with("xpub"));
    }

    #[test]
    fn test_create_payment_method_request_erc20() {
        let req = CreatePaymentMethodRequest {
            chain_id: 137,
            token_address: Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string()),
            asset_symbol: "USDC".to_string(),
            decimals: 6,
            xpub: "xpub123...".to_string(),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["chain_id"], 137);
        assert_eq!(json["token_address"], "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        assert_eq!(json["decimals"], 6);
    }

    #[test]
    fn test_update_payment_method_request_toggle_enabled() {
        let req = UpdatePaymentMethodRequest {
            enabled: Some(false),
            xpub: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["enabled"], false);
        assert!(json["xpub"].is_null());
    }

    #[test]
    fn test_update_payment_method_request_change_xpub() {
        let req = UpdatePaymentMethodRequest {
            enabled: None,
            xpub: Some("xpub6NEW...".to_string()),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert!(json["enabled"].is_null());
        assert_eq!(json["xpub"], "xpub6NEW...");
    }

    #[test]
    fn test_update_payment_method_request_both_fields() {
        let req = UpdatePaymentMethodRequest {
            enabled: Some(true),
            xpub: Some("xpub6ABC...".to_string()),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["enabled"], true);
        assert_eq!(json["xpub"], "xpub6ABC...");
    }

    #[test]
    fn test_store_webhook_serialization() {
        let wh = StoreWebhook {
            id: "wh_001".to_string(),
            store_id: "store_001".to_string(),
            webhook_url: "https://example.com/hook".to_string(),
            webhook_secret: Some("secret123".to_string()),
            enabled: true,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&wh).unwrap();
        let parsed: StoreWebhook = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.webhook_url, "https://example.com/hook");
        assert_eq!(parsed.webhook_secret, Some("secret123".to_string()));
        assert!(parsed.enabled);
    }

    #[test]
    fn test_store_webhook_from_backend_get() {
        // Backend GET returns webhook_secret as null
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "store_id": "660e8400-e29b-41d4-a716-446655440000",
            "webhook_url": "https://example.com/hook",
            "webhook_secret": null,
            "enabled": true,
            "created_at": "2024-06-15T10:30:00Z",
            "updated_at": "2024-06-15T10:30:00Z"
        }"#;
        let wh: StoreWebhook = serde_json::from_str(json).unwrap();
        assert!(wh.webhook_secret.is_none());
        assert!(wh.enabled);
    }

    #[test]
    fn test_store_webhook_from_backend_put() {
        // Backend PUT returns webhook_secret with the new secret
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "store_id": "660e8400-e29b-41d4-a716-446655440000",
            "webhook_url": "https://example.com/hook",
            "webhook_secret": "whsec_abc123",
            "enabled": true,
            "created_at": "2024-06-15T10:30:00Z",
            "updated_at": "2024-06-15T10:30:00Z"
        }"#;
        let wh: StoreWebhook = serde_json::from_str(json).unwrap();
        assert_eq!(wh.webhook_secret, Some("whsec_abc123".to_string()));
    }

    #[test]
    fn test_store_webhook_enabled_default() {
        let json = r#"{
            "id": "wh_002",
            "store_id": "s1",
            "webhook_url": "https://example.com/hook",
            "webhook_secret": null,
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;
        let wh: StoreWebhook = serde_json::from_str(json).unwrap();
        assert!(wh.enabled);
    }

    // =========================================================================
    // UpdateWebhookRequest
    // =========================================================================

    #[test]
    fn test_update_webhook_request_serialization() {
        let req = UpdateWebhookRequest {
            webhook_url: "https://example.com/hook".to_string(),
            enabled: true,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["webhook_url"], "https://example.com/hook");
        assert_eq!(json["enabled"], true);
    }

    #[test]
    fn test_update_webhook_request_disabled() {
        let req = UpdateWebhookRequest {
            webhook_url: "http://localhost:1234".to_string(),
            enabled: false,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["webhook_url"], "http://localhost:1234");
        assert_eq!(json["enabled"], false);
    }

    #[test]
    fn test_update_webhook_request_roundtrip() {
        let req = UpdateWebhookRequest {
            webhook_url: "https://api.example.com/webhooks/payments".to_string(),
            enabled: true,
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: UpdateWebhookRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.webhook_url, req.webhook_url);
        assert_eq!(parsed.enabled, req.enabled);
    }

    // =========================================================================
    // Paginated response
    // =========================================================================

    #[test]
    fn test_paginated_response() {
        let json = r#"{
            "data": [
                {"id": "inv_1", "store_id": "s1", "currency": "USD", "status": "pending", "amount": "100", "amount_received": "0", "created_at": "2024-01-01T00:00:00Z", "expires_at": "2024-01-02T00:00:00Z", "metadata": null}
            ],
            "total": 50,
            "page": 1,
            "per_page": 10
        }"#;
        let resp: PaginatedResponse<Invoice> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data.len(), 1);
        assert_eq!(resp.total, 50);
        assert_eq!(resp.page, 1);
        assert_eq!(resp.per_page, 10);
        assert_eq!(resp.data[0].id, "inv_1");
    }

    // =========================================================================
    // Create invoice request
    // =========================================================================

    #[test]
    fn test_create_invoice_request() {
        let req = CreateInvoiceRequest {
            store_id: "store_001".to_string(),
            amount: "99.99".to_string(),
            currency: "USD".to_string(),
            chain_id: Some(1),
            metadata: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["store_id"], "store_001");
        assert_eq!(json["amount"], "99.99");
        assert_eq!(json["currency"], "USD");
        assert_eq!(json["chain_id"], 1);
    }

    #[test]
    fn test_create_invoice_request_minimal() {
        let req = CreateInvoiceRequest {
            store_id: "s1".to_string(),
            amount: "10".to_string(),
            currency: "ETH".to_string(),
            chain_id: None,
            metadata: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert!(json["chain_id"].is_null());
        assert!(json["metadata"].is_null());
    }
}
