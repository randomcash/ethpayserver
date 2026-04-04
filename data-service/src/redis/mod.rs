//! Redis data service implementation.
//!
//! Provides fast, ephemeral storage for watched addresses that need quick
//! recovery on monitor restart. This is complementary to PostgreSQL storage -
//! Redis provides speed while PostgreSQL provides durability.

mod watched_address;

use redis::Client;
use redis::aio::ConnectionManager;

use crate::RepositoryError;

/// Redis data service for fast ephemeral storage.
///
/// Used primarily for watched address persistence to enable fast recovery
/// when evmmonitor instances restart.
pub struct RedisDataService {
    /// Connection manager (connection pool) for Redis operations.
    conn: ConnectionManager,
}

impl RedisDataService {
    /// Create a new Redis data service.
    ///
    /// # Arguments
    ///
    /// * `url` - Redis connection URL (e.g., "redis://localhost:6379")
    pub async fn new(url: &str) -> Result<Self, RepositoryError> {
        let client = Client::open(url)
            .map_err(|e| RepositoryError::Database(format!("redis client error: {}", e)))?;

        let conn = ConnectionManager::new(client)
            .await
            .map_err(|e| RepositoryError::Database(format!("redis connection error: {}", e)))?;

        Ok(Self { conn })
    }

    /// Health check for Redis connection.
    pub async fn health_check(&self) -> Result<(), RepositoryError> {
        let mut conn = self.conn.clone();
        let pong: String = redis::cmd("PING")
            .query_async(&mut conn)
            .await
            .map_err(|e| RepositoryError::Database(format!("redis ping failed: {}", e)))?;

        if pong == "PONG" {
            Ok(())
        } else {
            Err(RepositoryError::Database(
                "redis health check: unexpected response".to_string(),
            ))
        }
    }
}
