//! Event bridge abstraction for inter-process communication.
//!
//! The bridge enables monitor processes to publish events that can be
//! consumed by API servers or other services.

mod memory;
#[cfg(feature = "redis")]
mod redis;

pub use memory::MemoryBridge;
#[cfg(feature = "redis")]
pub use self::redis::RedisBridge;

use super::events::MonitorEvent;
use crate::error::EvmResult;
use async_trait::async_trait;
use std::pin::Pin;
use tokio_stream::Stream;

/// Stream of monitor events.
pub type EventStream = Pin<Box<dyn Stream<Item = MonitorEvent> + Send>>;

/// Event bridge for publishing and subscribing to monitor events.
///
/// Implementations can use different backends (Redis, NATS, in-memory, etc.)
/// to facilitate communication between the monitor binary and API servers.
#[async_trait]
pub trait EventBridge: Send + Sync {
    /// Publish an event to the bridge.
    ///
    /// Called by the monitor when a payment is detected, confirmed, etc.
    async fn publish(&self, event: &MonitorEvent) -> EvmResult<()>;

    /// Subscribe to events from the bridge.
    ///
    /// Called by API servers to receive events from all monitors.
    async fn subscribe(&self) -> EvmResult<EventStream>;

    /// Get the bridge name for logging.
    fn name(&self) -> &str;

    /// Check if the bridge is connected/healthy.
    async fn health_check(&self) -> EvmResult<()>;
}

/// Configuration for event bridges.
#[derive(Debug, Clone)]
pub enum BridgeConfig {
    /// In-memory bridge (for testing or single-process deployments).
    Memory,
    /// Redis pub/sub bridge.
    #[cfg(feature = "redis")]
    Redis {
        url: String,
        channel: String,
    },
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self::Memory
    }
}

impl BridgeConfig {
    /// Create a Redis bridge configuration.
    #[cfg(feature = "redis")]
    pub fn redis(url: impl Into<String>) -> Self {
        Self::Redis {
            url: url.into(),
            channel: "evmmonitor:events".to_string(),
        }
    }

    /// Create a Redis bridge with custom channel.
    #[cfg(feature = "redis")]
    pub fn redis_with_channel(url: impl Into<String>, channel: impl Into<String>) -> Self {
        Self::Redis {
            url: url.into(),
            channel: channel.into(),
        }
    }

    /// Create the bridge from this configuration.
    pub async fn build(self) -> EvmResult<Box<dyn EventBridge>> {
        match self {
            Self::Memory => Ok(Box::new(MemoryBridge::new())),
            #[cfg(feature = "redis")]
            Self::Redis { url, channel } => {
                let bridge = RedisBridge::new(&url, &channel).await?;
                Ok(Box::new(bridge))
            }
        }
    }
}
