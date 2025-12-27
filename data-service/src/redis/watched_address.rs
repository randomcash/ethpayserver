//! Live watched address repository implementation for Redis.
//!
//! This module provides runtime address watching for evmmonitor.
//! It stores simple `(chain_id, address) -> invoice_id` mappings
//! for fast lookups during transaction monitoring.
//!
//! Note: This is separate from the PostgreSQL `WatchedAddressReader/Writer`
//! traits which handle persistence with full invoice metadata.

use async_trait::async_trait;
use redis::AsyncCommands;
use tracing::warn;

use super::RedisDataService;
use crate::{LiveWatchedAddressReader, LiveWatchedAddressWriter, RepositoryError, RepositoryResult};
use types::InvoiceId;

/// Key prefix for watched addresses.
/// Format: `evmwatch:addr:{chain_id}:{address}` -> `invoice_id`
const KEY_PREFIX: &str = "evmwatch:addr";

impl RedisDataService {
    /// Build the Redis key for a watched address.
    fn watched_address_key(chain_id: u64, address: &str) -> String {
        format!("{}:{}:{}", KEY_PREFIX, chain_id, address.to_lowercase())
    }

    /// Parse a Redis key back into chain_id and address.
    fn parse_watched_address_key(key: &str) -> Option<(u64, String)> {
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
impl LiveWatchedAddressReader for RedisDataService {
    async fn get_watched_invoice(
        &self,
        address: &str,
        chain_id: u64,
    ) -> RepositoryResult<Option<InvoiceId>> {
        let key = Self::watched_address_key(chain_id, address);
        let mut conn = self.conn.clone();

        let result: Option<String> = conn
            .get(&key)
            .await
            .map_err(|e| RepositoryError::Database(format!("redis get failed: {}", e)))?;

        Ok(result.map(InvoiceId::from_string))
    }

    async fn get_all_watched(&self) -> RepositoryResult<Vec<(String, InvoiceId, u64)>> {
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
                    if let Some((chain_id, address)) = Self::parse_watched_address_key(&key) {
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
}

#[async_trait]
impl LiveWatchedAddressWriter for RedisDataService {
    async fn watch_address(
        &self,
        address: &str,
        invoice_id: &InvoiceId,
        chain_id: u64,
    ) -> RepositoryResult<()> {
        let key = Self::watched_address_key(chain_id, address);
        let mut conn = self.conn.clone();

        conn.set::<_, _, ()>(&key, invoice_id.as_str())
            .await
            .map_err(|e| RepositoryError::Database(format!("redis set failed: {}", e)))?;

        Ok(())
    }

    async fn unwatch_address(&self, address: &str, chain_id: u64) -> RepositoryResult<bool> {
        let key = Self::watched_address_key(chain_id, address);
        let mut conn = self.conn.clone();

        let deleted: i64 = conn
            .del(&key)
            .await
            .map_err(|e| RepositoryError::Database(format!("redis del failed: {}", e)))?;

        Ok(deleted > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_watched_address_key() {
        let key = RedisDataService::watched_address_key(
            1, // Ethereum mainnet
            "0x1234567890ABCDEF1234567890ABCDEF12345678",
        );
        assert_eq!(
            key,
            "evmwatch:addr:1:0x1234567890abcdef1234567890abcdef12345678"
        );
    }

    #[test]
    fn test_watched_address_key_testnet() {
        let key = RedisDataService::watched_address_key(
            11155111, // Sepolia
            "0x1234567890ABCDEF1234567890ABCDEF12345678",
        );
        assert_eq!(
            key,
            "evmwatch:addr:11155111:0x1234567890abcdef1234567890abcdef12345678"
        );
    }

    #[test]
    fn test_parse_watched_address_key() {
        let key = "evmwatch:addr:137:0xabcdef";
        let parsed = RedisDataService::parse_watched_address_key(key);
        assert!(parsed.is_some());
        let (chain_id, address) = parsed.unwrap();
        assert_eq!(chain_id, 137);
        assert_eq!(address, "0xabcdef");
    }

    #[test]
    fn test_parse_watched_address_key_testnet() {
        let key = "evmwatch:addr:11155111:0xabcdef";
        let parsed = RedisDataService::parse_watched_address_key(key);
        assert!(parsed.is_some());
        let (chain_id, address) = parsed.unwrap();
        assert_eq!(chain_id, 11155111);
        assert_eq!(address, "0xabcdef");
    }

    #[test]
    fn test_parse_invalid_key() {
        assert!(RedisDataService::parse_watched_address_key("invalid").is_none());
        assert!(RedisDataService::parse_watched_address_key("evmwatch:wrong:1:0x123").is_none());
        assert!(RedisDataService::parse_watched_address_key("evmwatch:addr").is_none());
    }
}
