//! Admin/settings API types.

use serde::{Deserialize, Serialize};
use types::ChainId;

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
    pub enabled_chain_ids: Vec<ChainId>,
}

/// Request to update server settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateServerSettingsRequest {
    pub default_confirmations: i32,
    pub invoice_expiry_minutes: i32,
    pub rate_limit_rpm: i32,
    pub enabled_chain_ids: Vec<ChainId>,
}

/// Request to update a user's role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateUserRoleRequest {
    pub role: String,
}
