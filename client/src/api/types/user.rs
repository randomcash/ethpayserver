//! User-related API types.

use serde::{Deserialize, Serialize};

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
