//! Wallet-related API types.

use serde::{Deserialize, Serialize};

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
