//! Role-based permission system for PayServer.
//!
//! This module implements a hierarchical permission system inspired by BTCPayServer.
//! Permissions are organized by scope (server, user) and roles grant sets of permissions.
//!
//! # Architecture
//!
//! - **Permission**: Individual capabilities like "can manage tokens" or "can view invoices"
//! - **Role**: Named collections of permissions (e.g., Admin, User)
//! - **Policies**: String constants for permission names (for API/storage)
//!
//! # Example
//!
//! ```rust
//! use auth::permissions::{Permission, Role};
//!
//! let admin = Role::ServerAdmin;
//! assert!(admin.has_permission(Permission::ServerManageTokens));
//! assert!(admin.has_permission(Permission::ServerViewSettings));
//!
//! let user = Role::User;
//! assert!(!user.has_permission(Permission::ServerManageTokens));
//! ```

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Permission policy string constants.
///
/// These follow the pattern `ethpay.{scope}.{action}` similar to BTCPayServer.
/// Scopes: `server`, `user`
pub struct Policies;

impl Policies {
    // =========================================================================
    // Server-level permissions (require admin role)
    // =========================================================================

    /// Can modify server settings (networks, RPC endpoints, etc.)
    pub const SERVER_MODIFY_SETTINGS: &'static str = "ethpay.server.canmodifyserversettings";

    /// Can manage tokens (add, remove, enable/disable)
    pub const SERVER_MANAGE_TOKENS: &'static str = "ethpay.server.canmanagetokens";

    /// Can view server settings
    pub const SERVER_VIEW_SETTINGS: &'static str = "ethpay.server.canviewserversettings";

    /// Can manage users (create, disable, change roles)
    pub const SERVER_MANAGE_USERS: &'static str = "ethpay.server.canmanageusers";

    /// Can view users
    pub const SERVER_VIEW_USERS: &'static str = "ethpay.server.canviewusers";

    // =========================================================================
    // User-level permissions (regular authenticated users)
    // =========================================================================

    /// Can view own profile
    pub const USER_VIEW_PROFILE: &'static str = "ethpay.user.canviewprofile";

    /// Can modify own profile
    pub const USER_MODIFY_PROFILE: &'static str = "ethpay.user.canmodifyprofile";

    /// Can create invoices
    pub const USER_CREATE_INVOICE: &'static str = "ethpay.user.cancreateinvoice";

    /// Can view own invoices
    pub const USER_VIEW_INVOICES: &'static str = "ethpay.user.canviewinvoices";

    /// Can view own payments
    pub const USER_VIEW_PAYMENTS: &'static str = "ethpay.user.canviewpayments";

    /// Unrestricted access (has all permissions)
    pub const UNRESTRICTED: &'static str = "unrestricted";

    /// Get all defined policies.
    pub fn all() -> &'static [&'static str] {
        &[
            Self::SERVER_MODIFY_SETTINGS,
            Self::SERVER_MANAGE_TOKENS,
            Self::SERVER_VIEW_SETTINGS,
            Self::SERVER_MANAGE_USERS,
            Self::SERVER_VIEW_USERS,
            Self::USER_VIEW_PROFILE,
            Self::USER_MODIFY_PROFILE,
            Self::USER_CREATE_INVOICE,
            Self::USER_VIEW_INVOICES,
            Self::USER_VIEW_PAYMENTS,
            Self::UNRESTRICTED,
        ]
    }

    /// Check if a policy string is valid.
    pub fn is_valid(policy: &str) -> bool {
        Self::all().contains(&policy)
    }

    /// Check if this is a server-level policy.
    pub fn is_server_policy(policy: &str) -> bool {
        policy.starts_with("ethpay.server")
    }

    /// Check if this is a user-level policy.
    pub fn is_user_policy(policy: &str) -> bool {
        policy.starts_with("ethpay.user")
    }
}

/// Individual permissions that can be granted to users.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    // Server permissions
    /// Can modify server settings
    ServerModifySettings,
    /// Can manage tokens
    ServerManageTokens,
    /// Can view server settings
    ServerViewSettings,
    /// Can manage users
    ServerManageUsers,
    /// Can view users
    ServerViewUsers,

    // User permissions
    /// Can view own profile
    UserViewProfile,
    /// Can modify own profile
    UserModifyProfile,
    /// Can create invoices
    UserCreateInvoice,
    /// Can view own invoices
    UserViewInvoices,
    /// Can view own payments
    UserViewPayments,

    /// Unrestricted - has all permissions
    Unrestricted,
}

impl Permission {
    /// Convert permission to policy string.
    pub fn as_policy(&self) -> &'static str {
        match self {
            Permission::ServerModifySettings => Policies::SERVER_MODIFY_SETTINGS,
            Permission::ServerManageTokens => Policies::SERVER_MANAGE_TOKENS,
            Permission::ServerViewSettings => Policies::SERVER_VIEW_SETTINGS,
            Permission::ServerManageUsers => Policies::SERVER_MANAGE_USERS,
            Permission::ServerViewUsers => Policies::SERVER_VIEW_USERS,
            Permission::UserViewProfile => Policies::USER_VIEW_PROFILE,
            Permission::UserModifyProfile => Policies::USER_MODIFY_PROFILE,
            Permission::UserCreateInvoice => Policies::USER_CREATE_INVOICE,
            Permission::UserViewInvoices => Policies::USER_VIEW_INVOICES,
            Permission::UserViewPayments => Policies::USER_VIEW_PAYMENTS,
            Permission::Unrestricted => Policies::UNRESTRICTED,
        }
    }

    /// Parse permission from policy string.
    pub fn from_policy(policy: &str) -> Option<Self> {
        match policy {
            Policies::SERVER_MODIFY_SETTINGS => Some(Permission::ServerModifySettings),
            Policies::SERVER_MANAGE_TOKENS => Some(Permission::ServerManageTokens),
            Policies::SERVER_VIEW_SETTINGS => Some(Permission::ServerViewSettings),
            Policies::SERVER_MANAGE_USERS => Some(Permission::ServerManageUsers),
            Policies::SERVER_VIEW_USERS => Some(Permission::ServerViewUsers),
            Policies::USER_VIEW_PROFILE => Some(Permission::UserViewProfile),
            Policies::USER_MODIFY_PROFILE => Some(Permission::UserModifyProfile),
            Policies::USER_CREATE_INVOICE => Some(Permission::UserCreateInvoice),
            Policies::USER_VIEW_INVOICES => Some(Permission::UserViewInvoices),
            Policies::USER_VIEW_PAYMENTS => Some(Permission::UserViewPayments),
            Policies::UNRESTRICTED => Some(Permission::Unrestricted),
            _ => None,
        }
    }

    /// Check if this permission implies another permission (hierarchy).
    ///
    /// For example, `ServerModifySettings` implies `ServerViewSettings`.
    pub fn implies(&self, other: Permission) -> bool {
        if *self == other {
            return true;
        }

        // Unrestricted implies everything
        if *self == Permission::Unrestricted {
            return true;
        }

        // Permission hierarchy
        match self {
            Permission::ServerModifySettings => matches!(
                other,
                Permission::ServerViewSettings
                    | Permission::ServerManageTokens
                    | Permission::ServerManageUsers
                    | Permission::ServerViewUsers
            ),
            Permission::ServerManageUsers => matches!(other, Permission::ServerViewUsers),
            Permission::UserModifyProfile => matches!(other, Permission::UserViewProfile),
            _ => false,
        }
    }
}

/// User roles with predefined permission sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Server administrator - has all permissions.
    ServerAdmin,

    /// Regular user - can manage own invoices and profile.
    #[default]
    User,
}

impl Role {
    /// Get the display name for this role.
    pub fn display_name(&self) -> &'static str {
        match self {
            Role::ServerAdmin => "Server Admin",
            Role::User => "User",
        }
    }

    /// Check if this role has a specific permission.
    pub fn has_permission(&self, permission: Permission) -> bool {
        match self {
            Role::ServerAdmin => true, // Admin has all permissions
            Role::User => {
                // Users have limited permissions
                matches!(
                    permission,
                    Permission::UserViewProfile
                        | Permission::UserModifyProfile
                        | Permission::UserCreateInvoice
                        | Permission::UserViewInvoices
                        | Permission::UserViewPayments
                )
            }
        }
    }

    /// Check if this role has permission via the policy string.
    pub fn has_policy(&self, policy: &str) -> bool {
        if let Some(permission) = Permission::from_policy(policy) {
            self.has_permission(permission)
        } else {
            false
        }
    }

    /// Get all permissions granted to this role.
    pub fn permissions(&self) -> Vec<Permission> {
        match self {
            Role::ServerAdmin => vec![
                Permission::ServerModifySettings,
                Permission::ServerManageTokens,
                Permission::ServerViewSettings,
                Permission::ServerManageUsers,
                Permission::ServerViewUsers,
                Permission::UserViewProfile,
                Permission::UserModifyProfile,
                Permission::UserCreateInvoice,
                Permission::UserViewInvoices,
                Permission::UserViewPayments,
                Permission::Unrestricted,
            ],
            Role::User => vec![
                Permission::UserViewProfile,
                Permission::UserModifyProfile,
                Permission::UserCreateInvoice,
                Permission::UserViewInvoices,
                Permission::UserViewPayments,
            ],
        }
    }

    /// Check if this role is a server admin.
    pub fn is_server_admin(&self) -> bool {
        matches!(self, Role::ServerAdmin)
    }

    /// Alias for `is_server_admin()` for convenience.
    pub fn is_admin(&self) -> bool {
        self.is_server_admin()
    }

    /// Check if this is a valid authenticated role.
    ///
    /// Returns true for any valid role (User or ServerAdmin).
    /// Use this to verify a user is authenticated regardless of their permission level.
    pub fn is_authenticated(&self) -> bool {
        // All defined roles are authenticated
        matches!(self, Role::ServerAdmin | Role::User)
    }

    /// Parse role from string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "serveradmin" | "server_admin" | "admin" => Some(Role::ServerAdmin),
            "user" => Some(Role::User),
            _ => None,
        }
    }

    /// Convert role to string for storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::ServerAdmin => "server_admin",
            Role::User => "user",
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Role {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Role::from_str(s).ok_or_else(|| format!("unknown role: {}", s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_admin_has_all_permissions() {
        let admin = Role::ServerAdmin;
        assert!(admin.has_permission(Permission::ServerModifySettings));
        assert!(admin.has_permission(Permission::ServerManageTokens));
        assert!(admin.has_permission(Permission::UserViewProfile));
        assert!(admin.has_permission(Permission::Unrestricted));
    }

    #[test]
    fn test_user_has_limited_permissions() {
        let user = Role::User;
        assert!(user.has_permission(Permission::UserViewProfile));
        assert!(user.has_permission(Permission::UserCreateInvoice));
        assert!(!user.has_permission(Permission::ServerModifySettings));
        assert!(!user.has_permission(Permission::ServerManageTokens));
        assert!(!user.has_permission(Permission::Unrestricted));
    }

    #[test]
    fn test_permission_implies() {
        assert!(Permission::Unrestricted.implies(Permission::ServerModifySettings));
        assert!(Permission::Unrestricted.implies(Permission::UserViewProfile));
        assert!(Permission::ServerModifySettings.implies(Permission::ServerViewSettings));
        assert!(Permission::ServerManageUsers.implies(Permission::ServerViewUsers));
        assert!(!Permission::UserViewProfile.implies(Permission::ServerModifySettings));
    }

    #[test]
    fn test_policy_strings() {
        assert_eq!(
            Permission::ServerManageTokens.as_policy(),
            "ethpay.server.canmanagetokens"
        );
        assert_eq!(
            Permission::from_policy("ethpay.server.canmanagetokens"),
            Some(Permission::ServerManageTokens)
        );
    }

    #[test]
    fn test_role_from_str() {
        assert_eq!(Role::from_str("admin"), Some(Role::ServerAdmin));
        assert_eq!(Role::from_str("server_admin"), Some(Role::ServerAdmin));
        assert_eq!(Role::from_str("user"), Some(Role::User));
        assert_eq!(Role::from_str("unknown"), None);
    }

    #[test]
    fn test_role_display() {
        assert_eq!(Role::ServerAdmin.to_string(), "server_admin");
        assert_eq!(Role::User.to_string(), "user");
    }

    #[test]
    fn test_policies_validation() {
        assert!(Policies::is_valid("ethpay.server.canmanagetokens"));
        assert!(Policies::is_valid("ethpay.user.canviewprofile"));
        assert!(!Policies::is_valid("invalid.policy"));
    }

    #[test]
    fn test_policy_scope() {
        assert!(Policies::is_server_policy("ethpay.server.canmanagetokens"));
        assert!(!Policies::is_server_policy("ethpay.user.canviewprofile"));
        assert!(Policies::is_user_policy("ethpay.user.canviewprofile"));
        assert!(!Policies::is_user_policy("ethpay.server.canmanagetokens"));
    }

    #[test]
    fn test_role_is_authenticated() {
        assert!(Role::ServerAdmin.is_authenticated());
        assert!(Role::User.is_authenticated());
    }

    #[test]
    fn test_role_is_admin() {
        assert!(Role::ServerAdmin.is_admin());
        assert!(!Role::User.is_admin());
        // is_admin is alias for is_server_admin
        assert_eq!(Role::ServerAdmin.is_admin(), Role::ServerAdmin.is_server_admin());
    }
}
