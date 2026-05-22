//! API key types.

use serde::{Deserialize, Serialize};

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
