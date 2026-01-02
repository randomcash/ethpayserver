//! Redis pub/sub event bridge.
//!
//! Uses Redis pub/sub for bidirectional communication between monitor
//! instances and API servers:
//!
//! - **Events channel**: Monitor publishes events, API servers subscribe
//! - **Commands channel**: API servers publish commands, monitors subscribe
//!
//! Supports multiple monitors publishing to the same channel and multiple
//! API servers subscribing.

use super::{CommandStream, EventBridge, EventStream};
use crate::error::{EvmError, EvmResult};
use crate::monitor::events::{MonitorCommand, MonitorEvent};
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
    /// Channel name for events (monitor -> API server).
    events_channel: String,
    /// Channel name for commands (API server -> monitor).
    commands_channel: String,
}

impl RedisBridge {
    /// Create a new Redis bridge.
    ///
    /// # Arguments
    ///
    /// * `url` - Redis connection URL (e.g., "redis://localhost:6379")
    /// * `events_channel` - Channel for events (e.g., "evmmonitor:events")
    /// * `commands_channel` - Channel for commands (e.g., "evmmonitor:commands")
    pub async fn new(url: &str, events_channel: &str, commands_channel: &str) -> EvmResult<Self> {
        let client = Client::open(url).map_err(|e| EvmError::Monitor(format!("redis connection failed: {}", e)))?;

        let publisher = ConnectionManager::new(client.clone())
            .await
            .map_err(|e| EvmError::Monitor(format!("redis connection manager failed: {}", e)))?;

        Ok(Self {
            client,
            publisher,
            events_channel: events_channel.to_string(),
            commands_channel: commands_channel.to_string(),
        })
    }

    /// Get the events channel name.
    pub fn events_channel(&self) -> &str {
        &self.events_channel
    }

    /// Get the commands channel name.
    pub fn commands_channel(&self) -> &str {
        &self.commands_channel
    }

    /// Get a value from Redis by key.
    pub async fn get_key(&self, key: &str) -> EvmResult<Option<String>> {
        let mut conn = self.publisher.clone();
        let value: Option<String> = conn.get(key)
            .await
            .map_err(|e| EvmError::Monitor(format!("redis GET failed: {}", e)))?;
        Ok(value)
    }
}

#[async_trait]
impl EventBridge for RedisBridge {
    // =========================================================================
    // Events (Monitor -> API Server)
    // =========================================================================

    async fn publish(&self, event: &MonitorEvent) -> EvmResult<()> {
        let payload = serde_json::to_string(event)
            .map_err(|e| EvmError::Monitor(format!("event serialization failed: {}", e)))?;

        let mut conn = self.publisher.clone();
        conn.publish::<_, _, ()>(&self.events_channel, &payload)
            .await
            .map_err(|e| EvmError::Monitor(format!("redis publish failed: {}", e)))?;

        debug!(channel = %self.events_channel, "published event to redis");
        Ok(())
    }

    async fn subscribe(&self) -> EvmResult<EventStream> {
        let mut pubsub = self
            .client
            .get_async_pubsub()
            .await
            .map_err(|e| EvmError::Monitor(format!("redis pubsub failed: {}", e)))?;

        pubsub
            .subscribe(&self.events_channel)
            .await
            .map_err(|e| EvmError::Monitor(format!("redis subscribe failed: {}", e)))?;

        let channel = self.events_channel.clone();
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
                        warn!(error = %e, payload = %payload, "failed to deserialize event");
                    }
                }
            }
            error!(channel = %channel, "redis events subscription ended unexpectedly");
        };

        Ok(Box::pin(stream))
    }

    // =========================================================================
    // Commands (API Server -> Monitor)
    // =========================================================================

    async fn publish_command(&self, command: &MonitorCommand) -> EvmResult<()> {
        let payload = serde_json::to_string(command)
            .map_err(|e| EvmError::Monitor(format!("command serialization failed: {}", e)))?;

        let mut conn = self.publisher.clone();
        conn.publish::<_, _, ()>(&self.commands_channel, &payload)
            .await
            .map_err(|e| EvmError::Monitor(format!("redis publish command failed: {}", e)))?;

        debug!(channel = %self.commands_channel, "published command to redis");
        Ok(())
    }

    async fn subscribe_commands(&self) -> EvmResult<CommandStream> {
        let mut pubsub = self
            .client
            .get_async_pubsub()
            .await
            .map_err(|e| EvmError::Monitor(format!("redis pubsub failed: {}", e)))?;

        pubsub
            .subscribe(&self.commands_channel)
            .await
            .map_err(|e| EvmError::Monitor(format!("redis subscribe commands failed: {}", e)))?;

        let channel = self.commands_channel.clone();
        let stream = async_stream::stream! {
            let mut msg_stream = pubsub.on_message();
            while let Some(msg) = msg_stream.next().await {
                let payload: String = match msg.get_payload() {
                    Ok(p) => p,
                    Err(e) => {
                        warn!(error = %e, "failed to get redis command payload");
                        continue;
                    }
                };

                match serde_json::from_str::<MonitorCommand>(&payload) {
                    Ok(command) => {
                        debug!(channel = %channel, "received command from redis");
                        yield command;
                    }
                    Err(e) => {
                        warn!(error = %e, payload = %payload, "failed to deserialize command");
                    }
                }
            }
            error!(channel = %channel, "redis commands subscription ended unexpectedly");
        };

        Ok(Box::pin(stream))
    }

    // =========================================================================
    // Utility
    // =========================================================================

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
