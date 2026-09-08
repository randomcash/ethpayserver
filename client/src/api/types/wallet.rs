//! Wallet-related API types.

use serde::{Deserialize, Serialize};

/// Wallet data from the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wallet {
    pub id: String,
    /// Owning account. Was `store_id` before RCS-234 moved wallets up a level:
    /// one xpub owns one derivation counter per account, and stores reference a
    /// wallet rather than owning one. Leaving this as `store_id` would not fail
    /// to compile - it would fail to deserialise, at runtime, on the page.
    pub user_id: String,
    pub xpub_masked: String,
    /// The next index this wallet will issue, across every store using it.
    pub derivation_index: i32,
    pub name: Option<String>,
    /// Whether stores with no override fall back to this wallet.
    pub is_primary: bool,
    pub created_at: String,
}
