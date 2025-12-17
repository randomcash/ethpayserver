//! Store role and permission models for multi-tenant payment processing.
//!
//! This module handles user roles and permissions within stores.
//! The `Store` model itself is defined in the `types` crate.
//!
//! # Architecture
//!
//! ```text
//! User ──── UserStore ──── Store (from types crate)
//!              │
//!              └── StoreRole (permissions)
//! ```
//!
//! - A **User** can belong to multiple **Stores**
//! - Each **UserStore** links a user to a store with a **StoreRole**
//! - **StoreRole** defines permissions (can be store-specific or global default)

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

// Re-export Store types from types crate
pub use types::{Store, StoreId, StoreInfo, UserId};

/// Unique identifier for a store role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
pub struct StoreRoleId(pub Uuid);

impl StoreRoleId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for StoreRoleId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for StoreRoleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Default store roles.
pub mod default_roles {
    /// Owner role - full access to the store.
    pub const OWNER: &str = "Owner";
    /// Manager role - can manage most store settings.
    pub const MANAGER: &str = "Manager";
    /// Employee role - can view and create invoices.
    pub const EMPLOYEE: &str = "Employee";
    /// Guest role - read-only access.
    pub const GUEST: &str = "Guest";
}

/// A role that defines permissions within a store.
///
/// Roles can be:
/// - **Global defaults**: `store_id` is None, available to all stores
/// - **Store-specific**: `store_id` is set, only available to that store
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreRole {
    /// Unique role identifier.
    pub id: StoreRoleId,

    /// Store this role belongs to (None for global default roles).
    pub store_id: Option<StoreId>,

    /// Role name (e.g., "Owner", "Manager", "Employee").
    pub role: String,

    /// List of permission policy strings granted by this role.
    /// Uses the same format as `Policies` (e.g., "ethpay.store.canviewinvoices").
    pub permissions: Vec<String>,
}

impl StoreRole {
    /// Create a new store-specific role.
    pub fn new(store_id: StoreId, role: impl Into<String>, permissions: Vec<String>) -> Self {
        Self {
            id: StoreRoleId::new(),
            store_id: Some(store_id),
            role: role.into(),
            permissions,
        }
    }

    /// Create a new global default role.
    pub fn new_default(role: impl Into<String>, permissions: Vec<String>) -> Self {
        Self {
            id: StoreRoleId::new(),
            store_id: None,
            role: role.into(),
            permissions,
        }
    }

    /// Check if this role has a specific permission.
    pub fn has_permission(&self, policy: &str) -> bool {
        self.permissions.iter().any(|p| p == policy)
    }
}

/// Sanitized store role information for API responses.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StoreRoleInfo {
    /// Unique role identifier.
    pub id: StoreRoleId,

    /// Store this role belongs to (None for global default roles).
    pub store_id: Option<StoreId>,

    /// Role name.
    pub role: String,

    /// List of permissions granted by this role.
    pub permissions: Vec<String>,
}

impl From<&StoreRole> for StoreRoleInfo {
    fn from(role: &StoreRole) -> Self {
        Self {
            id: role.id,
            store_id: role.store_id,
            role: role.role.clone(),
            permissions: role.permissions.clone(),
        }
    }
}

/// Links a user to a store with a specific role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserStore {
    /// User ID.
    pub user_id: UserId,

    /// Store ID.
    pub store_id: StoreId,

    /// Role ID defining permissions for this user in this store.
    pub store_role_id: StoreRoleId,
}

impl UserStore {
    /// Create a new user-store link.
    pub fn new(user_id: UserId, store_id: StoreId, store_role_id: StoreRoleId) -> Self {
        Self {
            user_id,
            store_id,
            store_role_id,
        }
    }
}

/// User's membership in a store with role details.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UserStoreInfo {
    /// Store information.
    pub store: StoreInfo,

    /// User's role in this store.
    pub role: StoreRoleInfo,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_role_permissions() {
        let store_id = StoreId::new();
        let role = StoreRole::new(
            store_id,
            "Manager",
            vec![
                "ethpay.store.canviewinvoices".to_string(),
                "ethpay.store.cancreateinvoice".to_string(),
            ],
        );

        assert!(role.has_permission("ethpay.store.canviewinvoices"));
        assert!(role.has_permission("ethpay.store.cancreateinvoice"));
        assert!(!role.has_permission("ethpay.store.canmodifysettings"));
    }

    #[test]
    fn test_default_role() {
        let role = StoreRole::new_default(
            default_roles::GUEST,
            vec!["ethpay.store.canviewinvoices".to_string()],
        );

        assert!(role.store_id.is_none());
        assert_eq!(role.role, "Guest");
    }

    #[test]
    fn test_store_creation() {
        let owner_id = UserId::new();
        let store = Store::new("My Store", owner_id)
            .with_website("https://example.com");

        assert_eq!(store.name, "My Store");
        assert_eq!(store.website, Some("https://example.com".to_string()));
        assert_eq!(store.owner_id, owner_id);
        assert!(!store.archived);
    }
}
