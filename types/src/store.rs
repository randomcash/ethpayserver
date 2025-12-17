//! Store models for multi-tenant payment processing.
//!
//! Stores represent merchant accounts that can process payments.
//! Role and permission management is handled by the `auth` crate.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use crate::types::UserId;

/// Unique identifier for a store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct StoreId(pub Uuid);

impl StoreId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for StoreId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for StoreId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A merchant store that can process payments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Store {
    /// Unique store identifier.
    pub id: StoreId,

    /// Store name (displayed to customers).
    pub name: String,

    /// Store website URL.
    pub website: Option<String>,

    /// Store owner user ID.
    pub owner_id: UserId,

    /// Whether the store is archived (soft delete).
    pub archived: bool,

    /// Store creation timestamp.
    pub created_at: DateTime<Utc>,
}

impl Store {
    /// Create a new store.
    pub fn new(name: impl Into<String>, owner_id: UserId) -> Self {
        Self {
            id: StoreId::new(),
            name: name.into(),
            website: None,
            owner_id,
            archived: false,
            created_at: Utc::now(),
        }
    }

    /// Set the store website.
    pub fn with_website(mut self, website: impl Into<String>) -> Self {
        self.website = Some(website.into());
        self
    }
}

/// Sanitized store information for API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct StoreInfo {
    /// Unique store identifier.
    pub id: StoreId,

    /// Store name.
    pub name: String,

    /// Store website URL.
    pub website: Option<String>,

    /// Whether the store is archived.
    pub archived: bool,

    /// Store creation timestamp.
    pub created_at: DateTime<Utc>,
}

impl From<&Store> for StoreInfo {
    fn from(store: &Store) -> Self {
        Self {
            id: store.id,
            name: store.name.clone(),
            website: store.website.clone(),
            archived: store.archived,
            created_at: store.created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
