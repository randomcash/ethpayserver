//! Redis pub/sub event bridge.
//!
//! Uses Redis pub/sub for communication between monitor instances
//! and API servers. Supports multiple monitors publishing to the same
//! channel and multiple API servers subscribing.

use super::{EventBridge, EventStream};
use crate::error::{EvmError, EvmResult};
use crate::monitor::events::MonitorEvent;
use async_trait::async_trait;
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client};
use tokio_stream::StreamExt;
use tracing::{debug, error, warn};

/// Redis pub/sub event bridge.
pub struct RedisBridge {
    /// Redis client for creating connections.
    client: Client,
    /// Connection manager for publishing (connection pool).
    publisher: ConnectionManager,
    /// Channel name for pub/sub.
    channel: String,
}

impl RedisBridge {
    /// Create a new Redis bridge.
    ///
    /// # Arguments
    ///
    /// * `url` - Redis connection URL (e.g., "redis://localhost:6379")
    /// * `channel` - Channel name for pub/sub (e.g., "evmmonitor:events")
    pub async fn new(url: &str, channel: &str) -> EvmResult<Self> {
        let client = Client::open(url).map_err(|e| EvmError::Monitor(format!("redis connection failed: {}", e)))?;

        let publisher = ConnectionManager::new(client.clone())
            .await
            .map_err(|e| EvmError::Monitor(format!("redis connection manager failed: {}", e)))?;

        Ok(Self {
            client,
            publisher,
            channel: channel.to_string(),
        })
    }

    /// Get the channel name.
    pub fn channel(&self) -> &str {
        &self.channel
    }
}

#[async_trait]
impl EventBridge for RedisBridge {
    async fn publish(&self, event: &MonitorEvent) -> EvmResult<()> {
        let payload = serde_json::to_string(event)
            .map_err(|e| EvmError::Monitor(format!("event serialization failed: {}", e)))?;

        let mut conn = self.publisher.clone();
        conn.publish::<_, _, ()>(&self.channel, &payload)
            .await
            .map_err(|e| EvmError::Monitor(format!("redis publish failed: {}", e)))?;

        debug!(channel = %self.channel, "published event to redis");
        Ok(())
    }

    async fn subscribe(&self) -> EvmResult<EventStream> {
        let mut pubsub = self
            .client
            .get_async_pubsub()
            .await
            .map_err(|e| EvmError::Monitor(format!("redis pubsub failed: {}", e)))?;

        pubsub
            .subscribe(&self.channel)
            .await
            .map_err(|e| EvmError::Monitor(format!("redis subscribe failed: {}", e)))?;

        let stream = async_stream::stream! {
            let mut msg_stream = pubsub.on_message();
            while let Some(msg) = msg_stream.next().await {
                let payload: String = match msg.get_payload() {
                    Ok(p) => p,
                    Err(e) => {
                        warn!(error = %e, "failed to get redis message payload");
                        continue;
                    }
                };

                match serde_json::from_str::<MonitorEvent>(&payload) {
                    Ok(event) => yield event,
                    Err(e) => {
                        warn!(error = %e, "failed to deserialize event");
                    }
                }
            }
            error!("redis subscription ended unexpectedly");
        };

        Ok(Box::pin(stream))
    }

    fn name(&self) -> &str {
        "RedisBridge"
    }

    async fn health_check(&self) -> EvmResult<()> {
        let mut conn = self.publisher.clone();
        let pong: String = redis::cmd("PING")
            .query_async(&mut conn)
            .await
            .map_err(|e| EvmError::Monitor(format!("redis health check failed: {}", e)))?;

        if pong == "PONG" {
            Ok(())
        } else {
            Err(EvmError::Monitor("redis health check: unexpected response".to_string()))
        }
    }
}

// Note: Integration tests for Redis require a running Redis instance.
// These are typically run in CI with a Redis service or skipped locally.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redis_url_parsing() {
        // Just verify URL parsing works
        let result = Client::open("redis://localhost:6379");
        assert!(result.is_ok());
    }
}
