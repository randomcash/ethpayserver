//! Watched address repository implementation for Redis.

use async_trait::async_trait;
use redis::AsyncCommands;
use tracing::warn;

use super::RedisDataService;
use crate::{
    CleanupAddressInfo, PendingWatchInfo, RepositoryError, RepositoryResult, WatchedAddressReader,
    WatchedAddressWriter,
};
use types::{InvoiceId, Network};

/// Key prefix for watched addresses.
/// Format: `evmwatch:addr:{network}:{address}` -> `invoice_id`
const KEY_PREFIX: &str = "evmwatch:addr";

impl RedisDataService {
    /// Build the Redis key for a watched address (using Network).
    fn watched_address_key(network: Network, address: &str) -> String {
        format!(
            "{}:{}:{}",
            KEY_PREFIX,
            network.as_str(),
            address.to_lowercase()
        )
    }

    /// Build the Redis key for a watched address (using chain_id).
    fn watched_address_key_by_chain(chain_id: u64, address: &str) -> String {
        format!("{}:{}:{}", KEY_PREFIX, chain_id, address.to_lowercase())
    }

    /// Parse a Redis key back into network and address.
    fn parse_watched_address_key(key: &str) -> Option<(Network, String)> {
        let parts: Vec<&str> = key.split(':').collect();
        // Expected format: evmwatch:addr:{network}:{address}
        if parts.len() != 4 || parts[0] != "evmwatch" || parts[1] != "addr" {
            return None;
        }

        let network: Network = parts[2].parse().ok()?;
        let address = parts[3].to_string();
        Some((network, address))
    }

    /// Parse a Redis key back into chain_id and address.
    fn parse_watched_address_key_by_chain(key: &str) -> Option<(u64, String)> {
        let parts: Vec<&str> = key.split(':').collect();
        // Expected format: evmwatch:addr:{chain_id}:{address}
        if parts.len() != 4 || parts[0] != "evmwatch" || parts[1] != "addr" {
            return None;
        }

        let chain_id: u64 = parts[2].parse().ok()?;
        let address = parts[3].to_string();
        Some((chain_id, address))
    }
}

#[async_trait]
impl WatchedAddressReader for RedisDataService {
    async fn get_invoice_id(
        &self,
        address: &str,
        network: Network,
    ) -> RepositoryResult<Option<InvoiceId>> {
        let key = Self::watched_address_key(network, address);
        let mut conn = self.conn.clone();

        let result: Option<String> = conn
            .get(&key)
            .await
            .map_err(|e| RepositoryError::Database(format!("redis get failed: {}", e)))?;

        Ok(result.map(InvoiceId::from_string))
    }

    async fn get_active(&self) -> RepositoryResult<Vec<(String, InvoiceId, Network)>> {
        let mut conn = self.conn.clone();
        let pattern = format!("{}:*", KEY_PREFIX);
        let mut result = Vec::new();

        // Use SCAN to iterate through all matching keys
        let mut cursor = 0u64;
        loop {
            let (new_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(100)
                .query_async(&mut conn)
                .await
                .map_err(|e| RepositoryError::Database(format!("redis scan failed: {}", e)))?;

            // Get values for all found keys
            for key in keys {
                let value: Option<String> = conn
                    .get(&key)
                    .await
                    .map_err(|e| RepositoryError::Database(format!("redis get failed: {}", e)))?;

                if let Some(invoice_id_str) = value {
                    if let Some((network, address)) = Self::parse_watched_address_key(&key) {
                        result.push((address, InvoiceId::from_string(invoice_id_str), network));
                    } else {
                        warn!(key = %key, "failed to parse watched address key");
                    }
                }
            }

            cursor = new_cursor;
            if cursor == 0 {
                break;
            }
        }

        Ok(result)
    }

    async fn get_pending(&self) -> RepositoryResult<Vec<PendingWatchInfo>> {
        // Redis implementation stores minimal data - pending info not tracked
        Ok(Vec::new())
    }

    async fn get_expired_for_cleanup(
        &self,
        _expiry_threshold_secs: i64,
    ) -> RepositoryResult<Vec<CleanupAddressInfo>> {
        // Redis implementation doesn't track expiry
        Ok(Vec::new())
    }

    async fn get_paid_for_cleanup(&self) -> RepositoryResult<Vec<CleanupAddressInfo>> {
        // Redis implementation doesn't track paid status
        Ok(Vec::new())
    }

    async fn get_cancelled_for_cleanup(&self) -> RepositoryResult<Vec<CleanupAddressInfo>> {
        // Redis implementation doesn't track cancelled status
        Ok(Vec::new())
    }
}

#[async_trait]
impl WatchedAddressWriter for RedisDataService {
    async fn upsert(
        &self,
        address: &str,
        invoice_id: &InvoiceId,
        network: Network,
    ) -> RepositoryResult<()> {
        let key = Self::watched_address_key(network, address);
        let mut conn = self.conn.clone();

        conn.set::<_, _, ()>(&key, invoice_id.as_str())
            .await
            .map_err(|e| RepositoryError::Database(format!("redis set failed: {}", e)))?;

        Ok(())
    }

    async fn remove(&self, address: &str, network: Network) -> RepositoryResult<()> {
        let key = Self::watched_address_key(network, address);
        let mut conn = self.conn.clone();

        let deleted: i64 = conn
            .del(&key)
            .await
            .map_err(|e| RepositoryError::Database(format!("redis del failed: {}", e)))?;

        if deleted == 0 {
            return Err(RepositoryError::NotFound(format!(
                "Watched address not found: {} on {:?}",
                address, network
            )));
        }

        Ok(())
    }

    async fn upsert_with_asset(
        &self,
        address: &str,
        invoice_id: &InvoiceId,
        network: Network,
        _asset_id: Option<&str>,
    ) -> RepositoryResult<()> {
        // Redis implementation ignores asset_id - just store the mapping
        self.upsert(address, invoice_id, network).await
    }

    async fn mark_notified(&self, _address: &str, _network: Network) -> RepositoryResult<()> {
        // Redis implementation doesn't track notification status
        Ok(())
    }

    async fn deactivate(&self, address: &str, network: Network) -> RepositoryResult<bool> {
        // For Redis, deactivate is the same as remove
        match self.remove(address, network).await {
            Ok(()) => Ok(true),
            Err(RepositoryError::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

/// Chain ID-based watched address methods for testnet support.
///
/// These methods work with chain_id directly instead of types::Network,
/// allowing support for testnets that aren't in the Network enum.
impl RedisDataService {
    /// Get the invoice ID for a watched address by chain ID.
    pub async fn get_invoice_id_by_chain(
        &self,
        address: &str,
        chain_id: u64,
    ) -> RepositoryResult<Option<InvoiceId>> {
        let key = Self::watched_address_key_by_chain(chain_id, address);
        let mut conn = self.conn.clone();

        let result: Option<String> = conn
            .get(&key)
            .await
            .map_err(|e| RepositoryError::Database(format!("redis get failed: {}", e)))?;

        Ok(result.map(InvoiceId::from_string))
    }

    /// Get all active watched addresses with chain IDs.
    pub async fn get_active_by_chain(&self) -> RepositoryResult<Vec<(String, InvoiceId, u64)>> {
        let mut conn = self.conn.clone();
        let pattern = format!("{}:*", KEY_PREFIX);
        let mut result = Vec::new();

        // Use SCAN to iterate through all matching keys
        let mut cursor = 0u64;
        loop {
            let (new_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(100)
                .query_async(&mut conn)
                .await
                .map_err(|e| RepositoryError::Database(format!("redis scan failed: {}", e)))?;

            // Get values for all found keys
            for key in keys {
                let value: Option<String> = conn
                    .get(&key)
                    .await
                    .map_err(|e| RepositoryError::Database(format!("redis get failed: {}", e)))?;

                if let Some(invoice_id_str) = value {
                    if let Some((chain_id, address)) =
                        Self::parse_watched_address_key_by_chain(&key)
                    {
                        result.push((address, InvoiceId::from_string(invoice_id_str), chain_id));
                    } else {
                        warn!(key = %key, "failed to parse watched address key");
                    }
                }
            }

            cursor = new_cursor;
            if cursor == 0 {
                break;
            }
        }

        Ok(result)
    }

    /// Register a watched address by chain ID.
    pub async fn upsert_by_chain(
        &self,
        address: &str,
        invoice_id: &InvoiceId,
        chain_id: u64,
    ) -> RepositoryResult<()> {
        let key = Self::watched_address_key_by_chain(chain_id, address);
        let mut conn = self.conn.clone();

        conn.set::<_, _, ()>(&key, invoice_id.as_str())
            .await
            .map_err(|e| RepositoryError::Database(format!("redis set failed: {}", e)))?;

        Ok(())
    }

    /// Remove a watched address by chain ID.
    pub async fn remove_by_chain(&self, address: &str, chain_id: u64) -> RepositoryResult<()> {
        let key = Self::watched_address_key_by_chain(chain_id, address);
        let mut conn = self.conn.clone();

        let deleted: i64 = conn
            .del(&key)
            .await
            .map_err(|e| RepositoryError::Database(format!("redis del failed: {}", e)))?;

        if deleted == 0 {
            return Err(RepositoryError::NotFound(format!(
                "Watched address not found: {} on chain {}",
                address, chain_id
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_watched_address_key() {
        let key = RedisDataService::watched_address_key(
            Network::Ethereum,
            "0x1234567890ABCDEF1234567890ABCDEF12345678",
        );
        assert_eq!(
            key,
            "evmwatch:addr:ethereum:0x1234567890abcdef1234567890abcdef12345678"
        );
    }

    #[test]
    fn test_parse_watched_address_key() {
        let key = "evmwatch:addr:polygon:0xabcdef";
        let parsed = RedisDataService::parse_watched_address_key(key);
        assert!(parsed.is_some());
        let (network, address) = parsed.unwrap();
        assert_eq!(network, Network::Polygon);
        assert_eq!(address, "0xabcdef");
    }

    #[test]
    fn test_parse_invalid_key() {
        assert!(RedisDataService::parse_watched_address_key("invalid").is_none());
        assert!(RedisDataService::parse_watched_address_key("evmwatch:wrong:eth:0x123").is_none());
        assert!(RedisDataService::parse_watched_address_key("evmwatch:addr").is_none());
    }

    #[test]
    fn test_watched_address_key_by_chain() {
        let key = RedisDataService::watched_address_key_by_chain(
            11155111, // Sepolia
            "0x1234567890ABCDEF1234567890ABCDEF12345678",
        );
        assert_eq!(
            key,
            "evmwatch:addr:11155111:0x1234567890abcdef1234567890abcdef12345678"
        );
    }

    #[test]
    fn test_parse_watched_address_key_by_chain() {
        let key = "evmwatch:addr:11155111:0xabcdef";
        let parsed = RedisDataService::parse_watched_address_key_by_chain(key);
        assert!(parsed.is_some());
        let (chain_id, address) = parsed.unwrap();
        assert_eq!(chain_id, 11155111);
        assert_eq!(address, "0xabcdef");
    }
}
