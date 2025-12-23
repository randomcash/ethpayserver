//! Event bridge abstraction for inter-process communication.
//!
//! The bridge enables bidirectional communication between monitor processes
//! and API servers:
//!
//! - **Events** flow from monitors to API servers (PaymentDetected, etc.)
//! - **Commands** flow from API servers to monitors (WatchAddress, etc.)

mod memory;
#[cfg(feature = "redis")]
mod redis;

pub use memory::MemoryBridge;
#[cfg(feature = "redis")]
pub use self::redis::RedisBridge;

use super::events::{MonitorCommand, MonitorEvent};
use crate::error::EvmResult;
use async_trait::async_trait;
use std::pin::Pin;
use tokio_stream::Stream;

/// Stream of monitor events (monitor -> API server).
pub type EventStream = Pin<Box<dyn Stream<Item = MonitorEvent> + Send>>;

/// Stream of monitor commands (API server -> monitor).
pub type CommandStream = Pin<Box<dyn Stream<Item = MonitorCommand> + Send>>;

/// Event bridge for bidirectional communication between monitors and API servers.
///
/// Implementations can use different backends (Redis, NATS, in-memory, etc.)
/// to facilitate communication between the monitor binary and API servers.
///
/// ## Channel Structure
///
/// - **Events channel** (`evmmonitor:events`): Monitor publishes, API server subscribes
/// - **Commands channel** (`evmmonitor:commands`): API server publishes, monitor subscribes
#[async_trait]
pub trait EventBridge: Send + Sync {
    // =========================================================================
    // Events (Monitor -> API Server)
    // =========================================================================

    /// Publish an event to the bridge.
    ///
    /// Called by the monitor when a payment is detected, confirmed, etc.
    async fn publish(&self, event: &MonitorEvent) -> EvmResult<()>;

    /// Subscribe to events from the bridge.
    ///
    /// Called by API servers to receive events from all monitors.
    async fn subscribe(&self) -> EvmResult<EventStream>;

    // =========================================================================
    // Commands (API Server -> Monitor)
    // =========================================================================

    /// Publish a command to the monitors.
    ///
    /// Called by API servers to instruct monitors (e.g., watch an address).
    async fn publish_command(&self, command: &MonitorCommand) -> EvmResult<()>;

    /// Subscribe to commands from the API server.
    ///
    /// Called by monitors to receive instructions (e.g., WatchAddress).
    async fn subscribe_commands(&self) -> EvmResult<CommandStream>;

    // =========================================================================
    // Utility
    // =========================================================================

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
        /// Channel for events (monitor -> API server).
        events_channel: String,
        /// Channel for commands (API server -> monitor).
        commands_channel: String,
    },
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self::Memory
    }
}

impl BridgeConfig {
    /// Create a Redis bridge configuration with default channels.
    #[cfg(feature = "redis")]
    pub fn redis(url: impl Into<String>) -> Self {
        Self::Redis {
            url: url.into(),
            events_channel: "evmmonitor:events".to_string(),
            commands_channel: "evmmonitor:commands".to_string(),
        }
    }

    /// Create a Redis bridge with custom channels.
    #[cfg(feature = "redis")]
    pub fn redis_with_channels(
        url: impl Into<String>,
        events_channel: impl Into<String>,
        commands_channel: impl Into<String>,
    ) -> Self {
        Self::Redis {
            url: url.into(),
            events_channel: events_channel.into(),
            commands_channel: commands_channel.into(),
        }
    }

    /// Create the bridge from this configuration.
    pub async fn build(self) -> EvmResult<Box<dyn EventBridge>> {
        match self {
            Self::Memory => Ok(Box::new(MemoryBridge::new())),
            #[cfg(feature = "redis")]
            Self::Redis { url, events_channel, commands_channel } => {
                let bridge = RedisBridge::new(&url, &events_channel, &commands_channel).await?;
                Ok(Box::new(bridge))
            }
        }
    }
}
