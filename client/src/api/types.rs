//! API types for ethpayserver.
//!
//! These types mirror the backend types from payserver-commons/types.

use serde::{Deserialize, Serialize};

// Re-export InvoiceStatus from the shared types crate.
pub use types::InvoiceStatus;

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

/// Payment data from the API.
///
/// Mirrors `PaymentResponse` from the backend.
/// A payment is an actual received transaction belonging to an invoice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payment {
    pub id: String,
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

/// Wallet data from the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wallet {
    pub id: String,
    pub store_id: String,
    pub xpub_masked: String,
    pub derivation_index: i32,
    pub name: Option<String>,
    pub created_at: String,
}

/// User role.
///
/// Mirrors `Role` from the auth crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    ServerAdmin,
    #[default]
    User,
}

impl UserRole {
    pub fn label(&self) -> &'static str {
        match self {
            UserRole::ServerAdmin => "Server Admin",
            UserRole::User => "User",
        }
    }

    pub fn is_admin(&self) -> bool {
        matches!(self, UserRole::ServerAdmin)
    }
}

/// Authenticated user info from `/auth/me`.
///
/// Mirrors `UserInfo` from the auth crate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub id: String,
    pub email: Option<String>,
    pub primary_wallet_address: Option<String>,
    pub created_at: String,
    pub last_login_at: Option<String>,
    pub role: UserRole,
}

/// Dashboard statistics.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DashboardStats {
    pub total_invoices: i64,
    pub pending_invoices: i64,
    pub paid_invoices: i64,
    pub expired_invoices: i64,
    pub total_payments: i64,
    pub total_stores: u32,
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

/// Paginated response wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
}

/// Invoice list response from the backend.
///
/// Mirrors `InvoiceListResponse` from the server API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceListResponse {
    pub total: i64,
    pub invoices: Vec<Invoice>,
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
    pub invoice: Invoice,
    pub payment: Payment,
}

/// API key info (returned for list/get).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyInfo {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub is_active: bool,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub expires_at: Option<String>,
    pub deprecated_at: Option<String>,
    /// Grace-window deadline for a deprecated key. Present only when the key
    /// is deprecated; the client renders this to show the exact expiry
    /// rather than assuming a hardcoded grace duration.
    #[serde(default)]
    pub deprecation_expires_at: Option<String>,
    #[serde(default)]
    pub rate_limit_rpm: Option<i32>,
}

/// Response after rotating an API key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotateApiKeyResponse {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub created_at: String,
    /// The new plaintext API key.
    pub key: String,
    pub old_key_deprecated_at: String,
    /// When the old key stops authenticating. Display this to the user
    /// instead of hardcoding a grace duration.
    #[serde(default)]
    pub old_key_grace_expires_at: Option<String>,
}

/// Response for listing API keys.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyListResponse {
    pub keys: Vec<ApiKeyInfo>,
}

/// Request to create a new API key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// Response after creating an API key (includes plaintext key).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateApiKeyResponsePayload {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub is_active: bool,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub key: String,
}

// =========================================================================
// Admin types
// =========================================================================

/// Admin user info (from GET /admin/users).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminUserInfo {
    pub id: String,
    pub email: Option<String>,
    pub primary_wallet_address: Option<String>,
    pub role: String,
    pub created_at: String,
    pub last_login_at: Option<String>,
    pub locked_until: Option<String>,
}

/// Paginated user list response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserListResponse {
    pub users: Vec<AdminUserInfo>,
    pub total: i64,
    pub offset: i64,
    pub limit: i64,
}

/// Server settings response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerSettingsResponse {
    pub default_confirmations: i32,
    pub invoice_expiry_minutes: i32,
    pub rate_limit_rpm: i32,
    pub enabled_chain_ids: Vec<i64>,
}

/// Request to update server settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateServerSettingsRequest {
    pub default_confirmations: i32,
    pub invoice_expiry_minutes: i32,
    pub rate_limit_rpm: i32,
    pub enabled_chain_ids: Vec<i64>,
}

/// Request to update a user's role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateUserRoleRequest {
    pub role: String,
}

/// Store settings (defaults, branding, notification prefs).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreSettings {
    pub store_id: String,
    pub default_chain_id: Option<i64>,
    pub default_display_currency: Option<String>,
    pub logo_url: Option<String>,
    pub accent_color: Option<String>,
    pub notification_prefs: serde_json::Value,
    pub updated_at: String,
}

/// Request to update store settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateStoreSettingsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_chain_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_display_currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notification_prefs: Option<serde_json::Value>,
}

/// Token policy entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPolicyEntry {
    pub chain_id: i64,
    pub token_address: Option<String>,
    pub asset_symbol: String,
}

/// Token policy response from the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPolicy {
    pub id: String,
    pub store_id: String,
    pub mode: String,
    pub entries: Vec<TokenPolicyEntry>,
    pub created_at: String,
    pub updated_at: String,
}

/// Request to set a token policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetTokenPolicyRequest {
    pub mode: String,
    pub entries: Vec<TokenPolicyEntry>,
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
            amount: "100.00".to_string(),
            currency: "USD".to_string(),
            status: InvoiceStatus::Pending,
            amount_received: "0.00".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            expires_at: "2024-01-02T00:00:00Z".to_string(),
            metadata: None,
            payment_options: vec![],
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
            chain_id: 1,
            invoice_id: "inv_001".to_string(),
            amount: "50000000000000000".to_string(),
            asset_symbol: "ETH".to_string(),
            token_address: None,
            tx_hash: "0xabc123...".to_string(),
            block_number: Some(19000000),
            detected_at: "2024-01-01T00:00:00Z".to_string(),
            confirmed_at: Some("2024-01-01T00:05:00Z".to_string()),
            from_address: Some("0x1234...".to_string()),
            reorged: false,
        };

        let json = serde_json::to_string(&payment).unwrap();
        let parsed: Payment = serde_json::from_str(&json).unwrap();

        assert_eq!(payment.id, parsed.id);
        assert_eq!(payment.chain_id, parsed.chain_id);
        assert_eq!(payment.invoice_id, parsed.invoice_id);
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
            store_id: "store_001".to_string(),
            xpub_masked: "xpub6CUG...Ht4QRnxv".to_string(),
            derivation_index: 3,
            name: Some("Main Wallet".to_string()),
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&wallet).unwrap();
        let parsed: Wallet = serde_json::from_str(&json).unwrap();

        assert_eq!(wallet.id, parsed.id);
        assert_eq!(wallet.store_id, parsed.store_id);
        assert_eq!(wallet.xpub_masked, parsed.xpub_masked);
        assert_eq!(wallet.derivation_index, parsed.derivation_index);
        assert_eq!(wallet.name, parsed.name);
    }

    #[test]
    fn test_dashboard_stats_default() {
        let stats = DashboardStats::default();

        assert_eq!(stats.total_invoices, 0);
        assert_eq!(stats.pending_invoices, 0);
        assert_eq!(stats.paid_invoices, 0);
        assert_eq!(stats.expired_invoices, 0);
        assert_eq!(stats.total_payments, 0);
        assert_eq!(stats.total_stores, 0);
    }

    #[test]
    fn test_dashboard_stats_deserialize_from_backend() {
        let json = serde_json::json!({
            "total_invoices": 42,
            "pending_invoices": 5,
            "paid_invoices": 30,
            "expired_invoices": 7,
            "total_payments": 35,
            "total_stores": 3
        });
        let stats: DashboardStats = serde_json::from_value(json).unwrap();
        assert_eq!(stats.total_invoices, 42);
        assert_eq!(stats.pending_invoices, 5);
        assert_eq!(stats.paid_invoices, 30);
        assert_eq!(stats.expired_invoices, 7);
        assert_eq!(stats.total_payments, 35);
        assert_eq!(stats.total_stores, 3);
    }

    #[test]
    fn test_dashboard_stats_roundtrip() {
        let stats = DashboardStats {
            total_invoices: 100,
            pending_invoices: 10,
            paid_invoices: 80,
            expired_invoices: 10,
            total_payments: 95,
            total_stores: 2,
        };
        let json = serde_json::to_value(&stats).unwrap();
        let parsed: DashboardStats = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.total_invoices, 100);
        assert_eq!(parsed.total_stores, 2);
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
        assert_eq!(
            InvoiceStatus::PartiallyPaid.css_class(),
            "badge badge-warning"
        );
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
        assert_eq!(default_invoice_status(), InvoiceStatus::Pending);
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
    // InvoiceStatusResponse
    // =========================================================================

    #[test]
    fn test_invoice_status_response_from_backend() {
        let json = r#"{
            "id": "inv-1",
            "status": "paid",
            "amount": "100.00",
            "amount_received": "100.00",
            "currency": "USD",
            "expires_at": "2024-01-02T00:00:00Z",
            "payment_count": 1,
            "confirmed_count": 1,
            "is_paid": true,
            "is_expired": false,
            "payment_options": [],
            "payments": [
                {
                    "id": "pay-1",
                    "chain_id": 1,
                    "invoice_id": "inv-1",
                    "tx_hash": "0xabc123",
                    "amount": "50000000000000000",
                    "asset_symbol": "ETH",
                    "token_address": null,
                    "block_number": 19000000,
                    "from_address": "0x1234",
                    "detected_at": "2024-01-01T10:00:00Z",
                    "confirmed_at": "2024-01-01T10:05:00Z",
                    "reorged": false
                }
            ]
        }"#;
        let resp: InvoiceStatusResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, "inv-1");
        assert_eq!(resp.status, InvoiceStatus::Paid);
        assert!(resp.is_paid);
        assert_eq!(resp.payments.len(), 1);
        assert_eq!(resp.payments[0].tx_hash, "0xabc123");
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
        assert_eq!(
            pm.token_address.as_deref(),
            Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")
        );
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
        assert_eq!(
            json["token_address"],
            "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
        );
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
                {"id": "inv_1", "currency": "USD", "status": "pending", "amount": "100", "amount_received": "0", "created_at": "2024-01-01T00:00:00Z", "expires_at": "2024-01-02T00:00:00Z", "metadata": null}
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

    // =========================================================================
    // InvoiceListResponse (backend response for GET /invoices)
    // =========================================================================

    #[test]
    fn test_invoice_list_response_from_backend() {
        let json = r#"{
            "total": 42,
            "invoices": [
                {
                    "id": "550e8400-e29b-41d4-a716-446655440000",
                    "currency": "USD",
                    "status": "paid",
                    "amount": "100.00",
                    "amount_received": "100.00",
                    "created_at": "2024-06-15T10:30:00Z",
                    "expires_at": "2024-06-16T10:30:00Z",
                    "metadata": {"order_id": "ORD-123"},
                    "payment_options": []
                },
                {
                    "id": "660e8400-e29b-41d4-a716-446655440000",
                    "currency": "ETH",
                    "status": "pending",
                    "amount": "0.5",
                    "amount_received": "0",
                    "created_at": "2024-06-15T11:00:00Z",
                    "expires_at": "2024-06-15T11:15:00Z",
                    "metadata": null,
                    "payment_options": []
                }
            ]
        }"#;
        let resp: InvoiceListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.total, 42);
        assert_eq!(resp.invoices.len(), 2);
        assert_eq!(resp.invoices[0].id, "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(resp.invoices[0].status, InvoiceStatus::Paid);
        assert_eq!(resp.invoices[0].amount, "100.00");
        assert_eq!(resp.invoices[1].status, InvoiceStatus::Pending);
    }

    #[test]
    fn test_invoice_list_response_empty() {
        let json = r#"{"total": 0, "invoices": []}"#;
        let resp: InvoiceListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.total, 0);
        assert!(resp.invoices.is_empty());
    }

    #[test]
    fn test_invoice_from_backend() {
        let json = r#"{
            "id": "inv-1",
            "currency": "USD",
            "status": "expired",
            "amount": "50.00",
            "amount_received": "0",
            "created_at": "2024-01-01T00:00:00Z",
            "expires_at": "2024-01-01T01:00:00Z",
            "metadata": null,
            "payment_options": []
        }"#;
        let invoice: Invoice = serde_json::from_str(json).unwrap();
        assert_eq!(invoice.id, "inv-1");
        assert_eq!(invoice.status, InvoiceStatus::Expired);
        assert!(invoice.payment_options.is_empty());
    }

    #[test]
    fn test_invoice_with_payment_options() {
        let json = r#"{
            "id": "inv-2",
            "currency": "USD",
            "status": "pending",
            "amount": "100.00",
            "amount_received": "0",
            "created_at": "2024-01-01T00:00:00Z",
            "expires_at": "2024-01-01T01:00:00Z",
            "metadata": null,
            "payment_options": [
                {
                    "id": "po-1",
                    "payment_method_id": "ETH-1",
                    "chain_id": 1,
                    "asset_symbol": "ETH",
                    "token_address": null,
                    "decimals": 18,
                    "payment_address": "0xabc123",
                    "amount": "28000000000000000",
                    "rate": "0.00028",
                    "is_active": true
                }
            ]
        }"#;
        let invoice: Invoice = serde_json::from_str(json).unwrap();
        assert_eq!(invoice.payment_options.len(), 1);
        assert_eq!(invoice.payment_options[0].chain_id, 1);
        assert_eq!(invoice.payment_options[0].asset_symbol, "ETH");
    }

    #[test]
    fn test_invoice_list_response_roundtrip() {
        let resp = InvoiceListResponse {
            total: 1,
            invoices: vec![Invoice {
                id: "inv-rt".to_string(),
                currency: "USD".to_string(),
                status: InvoiceStatus::Paid,
                amount: "25.00".to_string(),
                amount_received: "25.00".to_string(),
                created_at: "2024-01-01T00:00:00Z".to_string(),
                expires_at: "2024-01-02T00:00:00Z".to_string(),
                metadata: None,
                payment_options: vec![],
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: InvoiceListResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.total, 1);
        assert_eq!(parsed.invoices[0].id, "inv-rt");
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
            expiration_seconds: Some(1800),
            metadata: None,
            customer_email: None,
            webhook_url: None,
            redirect_url: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["store_id"], "store_001");
        assert_eq!(json["amount"], "99.99");
        assert_eq!(json["currency"], "USD");
        assert_eq!(json["expiration_seconds"], 1800);
        // skip_serializing_if = None fields should be absent
        assert!(json.get("metadata").is_none());
        assert!(json.get("webhook_url").is_none());
    }

    #[test]
    fn test_create_invoice_request_minimal() {
        let req = CreateInvoiceRequest {
            store_id: "s1".to_string(),
            amount: "10".to_string(),
            currency: "ETH".to_string(),
            expiration_seconds: None,
            metadata: None,
            customer_email: None,
            webhook_url: None,
            redirect_url: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["store_id"], "s1");
        // Optional fields should not be present in JSON
        assert!(json.get("expiration_seconds").is_none());
        assert!(json.get("metadata").is_none());
    }

    #[test]
    fn test_user_role_default_is_user() {
        assert_eq!(UserRole::default(), UserRole::User);
    }

    #[test]
    fn test_user_role_labels() {
        assert_eq!(UserRole::ServerAdmin.label(), "Server Admin");
        assert_eq!(UserRole::User.label(), "User");
    }

    #[test]
    fn test_user_role_is_admin() {
        assert!(UserRole::ServerAdmin.is_admin());
        assert!(!UserRole::User.is_admin());
    }

    #[test]
    fn test_user_role_serde_roundtrip() {
        let admin_json = serde_json::to_value(UserRole::ServerAdmin).unwrap();
        assert_eq!(admin_json, serde_json::json!("server_admin"));

        let user_json = serde_json::to_value(UserRole::User).unwrap();
        assert_eq!(user_json, serde_json::json!("user"));

        let parsed: UserRole = serde_json::from_str("\"server_admin\"").unwrap();
        assert_eq!(parsed, UserRole::ServerAdmin);

        let parsed: UserRole = serde_json::from_str("\"user\"").unwrap();
        assert_eq!(parsed, UserRole::User);
    }

    #[test]
    fn test_user_info_deserialize_full() {
        let json = serde_json::json!({
            "id": "usr_123",
            "email": "alice@example.com",
            "primary_wallet_address": "0xabc",
            "created_at": "2026-01-01T00:00:00Z",
            "last_login_at": "2026-04-04T12:00:00Z",
            "role": "server_admin"
        });
        let user: UserInfo = serde_json::from_value(json).unwrap();
        assert_eq!(user.id, "usr_123");
        assert_eq!(user.email.as_deref(), Some("alice@example.com"));
        assert_eq!(user.primary_wallet_address.as_deref(), Some("0xabc"));
        assert_eq!(user.last_login_at.as_deref(), Some("2026-04-04T12:00:00Z"));
        assert!(user.role.is_admin());
    }

    #[test]
    fn test_user_info_deserialize_minimal() {
        let json = serde_json::json!({
            "id": "usr_456",
            "email": null,
            "primary_wallet_address": null,
            "created_at": "2026-03-15T10:00:00Z",
            "last_login_at": null,
            "role": "user"
        });
        let user: UserInfo = serde_json::from_value(json).unwrap();
        assert_eq!(user.id, "usr_456");
        assert!(user.email.is_none());
        assert!(user.primary_wallet_address.is_none());
        assert!(user.last_login_at.is_none());
        assert!(!user.role.is_admin());
    }
}
