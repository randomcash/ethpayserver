//! API key authentication for the MCP server.
//!
//! The server authenticates once at start-up against `ETHPAY_API_KEY` and then
//! serves every subsequent tool call as that key's owner, scoped to the stores
//! the owner can reach. `validate_api_key` is that single gate: it resolves the
//! raw key to a `(UserId, Vec<StoreId>)` session scope, which
//! `EthpayMcpServer::authorize_store` then enforces on every tool call.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use auth::{ApiKeyRepository, StoreRepository, UserId};
use types::StoreId;

/// The repositories needed to authenticate an API key and resolve its store scope.
///
/// Expressed as a supertrait bundle rather than naming a concrete data service
/// so the auth flow can be driven against a stub repository in tests.
pub trait McpAuthRepository: ApiKeyRepository + StoreRepository {}

impl<T: ApiKeyRepository + StoreRepository> McpAuthRepository for T {}

/// Hash a raw API key the way the `auth` crate stores it: SHA-256, hex-encoded.
pub fn hash_api_key(raw_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw_key.as_bytes());
    hex::encode(hasher.finalize())
}

/// Validate an API key and return the owning user ID and their accessible store IDs.
pub async fn validate_api_key(
    repo: &dyn McpAuthRepository,
    raw_key: &str,
) -> Result<(UserId, Vec<StoreId>)> {
    let key_hash = hash_api_key(raw_key);

    // Look up the key
    let api_key = repo
        .get_api_key_by_hash(&key_hash)
        .await
        .context("Database error looking up API key")?
        .context("Invalid API key")?;

    if !api_key.is_active {
        bail!("API key is deactivated");
    }

    if let Some(expires_at) = api_key.expires_at
        && expires_at < chrono::Utc::now()
    {
        bail!("API key has expired");
    }

    // Update last_used timestamp
    let _ = repo.update_last_used(api_key.id).await;

    let user_id = api_key.user_id;

    // Get all stores the user has access to
    let stores = repo
        .get_stores_for_user(user_id)
        .await
        .context("Failed to fetch user stores")?;
    let store_ids: Vec<StoreId> = stores.into_iter().map(|s| s.id).collect();

    Ok((user_id, store_ids))
}

#[cfg(test)]
mod tests;
