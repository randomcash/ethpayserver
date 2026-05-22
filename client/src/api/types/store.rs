//! Store-related API types.

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
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

/// Request to set a token policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetTokenPolicyRequest {
    pub mode: String,
    pub entries: Vec<TokenPolicyEntry>,
}
