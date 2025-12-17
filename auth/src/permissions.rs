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
    // Store-level permissions (scoped to specific stores)
    // =========================================================================

    /// Can modify store settings
    pub const STORE_MODIFY_SETTINGS: &'static str = "ethpay.store.canmodifystoresettings";

    /// Can view store settings
    pub const STORE_VIEW_SETTINGS: &'static str = "ethpay.store.canviewstoresettings";

    /// Can create invoices in store
    pub const STORE_CREATE_INVOICE: &'static str = "ethpay.store.cancreateinvoice";

    /// Can view invoices in store
    pub const STORE_VIEW_INVOICES: &'static str = "ethpay.store.canviewinvoices";

    /// Can modify invoices in store
    pub const STORE_MODIFY_INVOICES: &'static str = "ethpay.store.canmodifyinvoices";

    /// Can view payments in store
    pub const STORE_VIEW_PAYMENTS: &'static str = "ethpay.store.canviewpayments";

    /// Can manage webhooks in store
    pub const STORE_MANAGE_WEBHOOKS: &'static str = "ethpay.store.canmanagewebhooks";

    // =========================================================================
    // User-level permissions (personal, not store-scoped)
    // =========================================================================

    /// Can view own profile
    pub const USER_VIEW_PROFILE: &'static str = "ethpay.user.canviewprofile";

    /// Can modify own profile
    pub const USER_MODIFY_PROFILE: &'static str = "ethpay.user.canmodifyprofile";

    /// Can delete own account
    pub const USER_DELETE_ACCOUNT: &'static str = "ethpay.user.candeleteaccount";

    /// Can manage own notifications
    pub const USER_MANAGE_NOTIFICATIONS: &'static str = "ethpay.user.canmanagenotifications";

    /// Unrestricted access (has all permissions)
    pub const UNRESTRICTED: &'static str = "unrestricted";

    /// Get all defined policies.
    pub fn all() -> &'static [&'static str] {
        &[
            // Server policies
            Self::SERVER_MODIFY_SETTINGS,
            Self::SERVER_MANAGE_TOKENS,
            Self::SERVER_VIEW_SETTINGS,
            Self::SERVER_MANAGE_USERS,
            Self::SERVER_VIEW_USERS,
            // Store policies
            Self::STORE_MODIFY_SETTINGS,
            Self::STORE_VIEW_SETTINGS,
            Self::STORE_CREATE_INVOICE,
            Self::STORE_VIEW_INVOICES,
            Self::STORE_MODIFY_INVOICES,
            Self::STORE_VIEW_PAYMENTS,
            Self::STORE_MANAGE_WEBHOOKS,
            // User policies
            Self::USER_VIEW_PROFILE,
            Self::USER_MODIFY_PROFILE,
            Self::USER_DELETE_ACCOUNT,
            Self::USER_MANAGE_NOTIFICATIONS,
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

    /// Check if this is a store-level policy (can be scoped to a store).
    pub fn is_store_policy(policy: &str) -> bool {
        policy.starts_with("ethpay.store")
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
    // Server permissions (global, not store-scoped)
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

    // Store permissions (scoped to specific stores)
    /// Can modify store settings
    StoreModifySettings,
    /// Can view store settings
    StoreViewSettings,
    /// Can create invoices in store
    StoreCreateInvoice,
    /// Can view invoices in store
    StoreViewInvoices,
    /// Can modify invoices in store
    StoreModifyInvoices,
    /// Can view payments in store
    StoreViewPayments,
    /// Can manage webhooks in store
    StoreManageWebhooks,

    // User permissions (personal, not store-scoped)
    /// Can view own profile
    UserViewProfile,
    /// Can modify own profile
    UserModifyProfile,
    /// Can delete own account
    UserDeleteAccount,
    /// Can manage own notifications
    UserManageNotifications,

    /// Unrestricted - has all permissions
    Unrestricted,
}

impl Permission {
    /// Convert permission to policy string.
    pub fn as_policy(&self) -> &'static str {
        match self {
            // Server
            Permission::ServerModifySettings => Policies::SERVER_MODIFY_SETTINGS,
            Permission::ServerManageTokens => Policies::SERVER_MANAGE_TOKENS,
            Permission::ServerViewSettings => Policies::SERVER_VIEW_SETTINGS,
            Permission::ServerManageUsers => Policies::SERVER_MANAGE_USERS,
            Permission::ServerViewUsers => Policies::SERVER_VIEW_USERS,
            // Store
            Permission::StoreModifySettings => Policies::STORE_MODIFY_SETTINGS,
            Permission::StoreViewSettings => Policies::STORE_VIEW_SETTINGS,
            Permission::StoreCreateInvoice => Policies::STORE_CREATE_INVOICE,
            Permission::StoreViewInvoices => Policies::STORE_VIEW_INVOICES,
            Permission::StoreModifyInvoices => Policies::STORE_MODIFY_INVOICES,
            Permission::StoreViewPayments => Policies::STORE_VIEW_PAYMENTS,
            Permission::StoreManageWebhooks => Policies::STORE_MANAGE_WEBHOOKS,
            // User
            Permission::UserViewProfile => Policies::USER_VIEW_PROFILE,
            Permission::UserModifyProfile => Policies::USER_MODIFY_PROFILE,
            Permission::UserDeleteAccount => Policies::USER_DELETE_ACCOUNT,
            Permission::UserManageNotifications => Policies::USER_MANAGE_NOTIFICATIONS,
            Permission::Unrestricted => Policies::UNRESTRICTED,
        }
    }

    /// Parse permission from policy string.
    pub fn from_policy(policy: &str) -> Option<Self> {
        match policy {
            // Server
            Policies::SERVER_MODIFY_SETTINGS => Some(Permission::ServerModifySettings),
            Policies::SERVER_MANAGE_TOKENS => Some(Permission::ServerManageTokens),
            Policies::SERVER_VIEW_SETTINGS => Some(Permission::ServerViewSettings),
            Policies::SERVER_MANAGE_USERS => Some(Permission::ServerManageUsers),
            Policies::SERVER_VIEW_USERS => Some(Permission::ServerViewUsers),
            // Store
            Policies::STORE_MODIFY_SETTINGS => Some(Permission::StoreModifySettings),
            Policies::STORE_VIEW_SETTINGS => Some(Permission::StoreViewSettings),
            Policies::STORE_CREATE_INVOICE => Some(Permission::StoreCreateInvoice),
            Policies::STORE_VIEW_INVOICES => Some(Permission::StoreViewInvoices),
            Policies::STORE_MODIFY_INVOICES => Some(Permission::StoreModifyInvoices),
            Policies::STORE_VIEW_PAYMENTS => Some(Permission::StoreViewPayments),
            Policies::STORE_MANAGE_WEBHOOKS => Some(Permission::StoreManageWebhooks),
            // User
            Policies::USER_VIEW_PROFILE => Some(Permission::UserViewProfile),
            Policies::USER_MODIFY_PROFILE => Some(Permission::UserModifyProfile),
            Policies::USER_DELETE_ACCOUNT => Some(Permission::UserDeleteAccount),
            Policies::USER_MANAGE_NOTIFICATIONS => Some(Permission::UserManageNotifications),
            Policies::UNRESTRICTED => Some(Permission::Unrestricted),
            _ => None,
        }
    }

    /// Check if this permission is store-scoped.
    pub fn is_store_scoped(&self) -> bool {
        Policies::is_store_policy(self.as_policy())
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
            // Server hierarchy
            Permission::ServerModifySettings => matches!(
                other,
                Permission::ServerViewSettings
                    | Permission::ServerManageTokens
                    | Permission::ServerManageUsers
                    | Permission::ServerViewUsers
            ),
            Permission::ServerManageUsers => matches!(other, Permission::ServerViewUsers),

            // Store hierarchy
            Permission::StoreModifySettings => matches!(
                other,
                Permission::StoreViewSettings
                    | Permission::StoreModifyInvoices
                    | Permission::StoreManageWebhooks
            ),
            Permission::StoreModifyInvoices => matches!(
                other,
                Permission::StoreViewInvoices | Permission::StoreCreateInvoice
            ),
            Permission::StoreViewSettings => matches!(
                other,
                Permission::StoreViewInvoices | Permission::StoreViewPayments
            ),

            // User hierarchy
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
    ///
    /// Note: Store-scoped permissions (like `StoreCreateInvoice`) are not granted
    /// by the Role directly. They come from `StoreRole` via the `UserStore` relationship.
    /// This method only checks server-level and user-level permissions.
    pub fn has_permission(&self, permission: Permission) -> bool {
        match self {
            Role::ServerAdmin => true, // Admin has all permissions
            Role::User => {
                // Users have user-level permissions only
                // Store permissions come from StoreRole, not Role
                matches!(
                    permission,
                    Permission::UserViewProfile
                        | Permission::UserModifyProfile
                        | Permission::UserDeleteAccount
                        | Permission::UserManageNotifications
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
    ///
    /// Note: Store-scoped permissions are not included here. They come from
    /// `StoreRole` via the `UserStore` relationship.
    pub fn permissions(&self) -> Vec<Permission> {
        match self {
            Role::ServerAdmin => vec![
                // Server permissions
                Permission::ServerModifySettings,
                Permission::ServerManageTokens,
                Permission::ServerViewSettings,
                Permission::ServerManageUsers,
                Permission::ServerViewUsers,
                // User permissions
                Permission::UserViewProfile,
                Permission::UserModifyProfile,
                Permission::UserDeleteAccount,
                Permission::UserManageNotifications,
                // Unrestricted
                Permission::Unrestricted,
            ],
            Role::User => vec![
                Permission::UserViewProfile,
                Permission::UserModifyProfile,
                Permission::UserDeleteAccount,
                Permission::UserManageNotifications,
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
        // User has user-level permissions
        assert!(user.has_permission(Permission::UserViewProfile));
        assert!(user.has_permission(Permission::UserModifyProfile));
        assert!(user.has_permission(Permission::UserDeleteAccount));
        // User doesn't have server permissions
        assert!(!user.has_permission(Permission::ServerModifySettings));
        assert!(!user.has_permission(Permission::ServerManageTokens));
        assert!(!user.has_permission(Permission::Unrestricted));
        // Store permissions come from StoreRole, not Role
        assert!(!user.has_permission(Permission::StoreCreateInvoice));
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
