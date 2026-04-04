//! In-memory event bridge using tokio broadcast channels.
//!
//! Useful for testing and single-process deployments where the monitor
//! runs in the same process as the API server.
//!
//! Supports bidirectional communication:
//! - Events flow from monitor to API server
//! - Commands flow from API server to monitor

use super::{CommandStream, EventBridge, EventStream};
use crate::error::EvmResult;
use crate::monitor::events::{MonitorCommand, MonitorEvent};
use async_trait::async_trait;
use tokio::sync::broadcast;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

/// In-memory event bridge using tokio broadcast channels.
pub struct MemoryBridge {
    /// Events channel (monitor -> API server).
    events_tx: broadcast::Sender<MonitorEvent>,
    /// Commands channel (API server -> monitor).
    commands_tx: broadcast::Sender<MonitorCommand>,
}

impl MemoryBridge {
    /// Create a new in-memory bridge.
    pub fn new() -> Self {
        Self::with_capacity(4096)
    }

    /// Create a new in-memory bridge with specified capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let (events_tx, _) = broadcast::channel(capacity);
        let (commands_tx, _) = broadcast::channel(capacity);
        Self {
            events_tx,
            commands_tx,
        }
    }

    /// Get a raw broadcast sender for events (for direct use).
    pub fn events_sender(&self) -> broadcast::Sender<MonitorEvent> {
        self.events_tx.clone()
    }

    /// Get a raw broadcast sender for commands (for direct use).
    pub fn commands_sender(&self) -> broadcast::Sender<MonitorCommand> {
        self.commands_tx.clone()
    }
}

impl Default for MemoryBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventBridge for MemoryBridge {
    // =========================================================================
    // Events (Monitor -> API Server)
    // =========================================================================

    async fn publish(&self, event: &MonitorEvent) -> EvmResult<()> {
        // Ignore send errors (no receivers is fine)
        let _ = self.events_tx.send(event.clone());
        Ok(())
    }

    async fn subscribe(&self) -> EvmResult<EventStream> {
        let rx = self.events_tx.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(|result| result.ok());
        Ok(Box::pin(stream))
    }

    // =========================================================================
    // Commands (API Server -> Monitor)
    // =========================================================================

    async fn publish_command(&self, command: &MonitorCommand) -> EvmResult<()> {
        // Ignore send errors (no receivers is fine)
        let _ = self.commands_tx.send(command.clone());
        Ok(())
    }

    async fn subscribe_commands(&self) -> EvmResult<CommandStream> {
        let rx = self.commands_tx.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(|result| result.ok());
        Ok(Box::pin(stream))
    }

    // =========================================================================
    // Utility
    // =========================================================================

    fn name(&self) -> &str {
        "MemoryBridge"
    }

    async fn health_check(&self) -> EvmResult<()> {
        // Always healthy
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::events::{PaymentDetected, WatchAddressCommand};
    use alloy::primitives::{Address, B256, U256};
    use chrono::Utc;
    use tokio_stream::StreamExt;

    fn make_event() -> MonitorEvent {
        MonitorEvent::PaymentDetected(PaymentDetected {
            chain_id: 1,
            invoice_id: uuid::Uuid::new_v4(),
            payment_address: Address::ZERO,
            amount: U256::from(1000),
            tx_hash: B256::ZERO,
            block_number: 100,
            block_hash: B256::ZERO,
            log_index: None,
            is_native: true,
            token_address: None,
            from_address: Address::ZERO,
            confirmations: 1,
            required_confirmations: 12,
            detected_at: Utc::now(),
        })
    }

    fn make_command() -> MonitorCommand {
        MonitorCommand::WatchAddress(WatchAddressCommand {
            chain_id: 1,
            address: Address::ZERO,
            invoice_id: uuid::Uuid::new_v4(),
            expected_amount: Some(U256::from(1000)),
            token_contract: None,
        })
    }

    #[tokio::test]
    async fn test_memory_bridge_events_pubsub() {
        let bridge = MemoryBridge::new();

        // Subscribe first
        let mut stream = bridge.subscribe().await.unwrap();

        // Publish event
        let event = make_event();
        bridge.publish(&event).await.unwrap();

        // Receive event
        let received = tokio::time::timeout(std::time::Duration::from_millis(100), stream.next())
            .await
            .unwrap()
            .unwrap();

        match received {
            MonitorEvent::PaymentDetected(p) => {
                assert_eq!(p.chain_id, 1);
            }
            _ => panic!("unexpected event type"),
        }
    }

    #[tokio::test]
    async fn test_memory_bridge_commands_pubsub() {
        let bridge = MemoryBridge::new();

        // Subscribe to commands first
        let mut stream = bridge.subscribe_commands().await.unwrap();

        // Publish command
        let command = make_command();
        bridge.publish_command(&command).await.unwrap();

        // Receive command
        let received = tokio::time::timeout(std::time::Duration::from_millis(100), stream.next())
            .await
            .unwrap()
            .unwrap();

        match received {
            MonitorCommand::WatchAddress(w) => {
                assert_eq!(w.chain_id, 1);
            }
            _ => panic!("unexpected command type"),
        }
    }

    #[tokio::test]
    async fn test_memory_bridge_multiple_subscribers() {
        let bridge = MemoryBridge::new();

        let mut stream1 = bridge.subscribe().await.unwrap();
        let mut stream2 = bridge.subscribe().await.unwrap();

        let event = make_event();
        bridge.publish(&event).await.unwrap();

        // Both should receive
        let r1 = stream1.next().await;
        let r2 = stream2.next().await;

        assert!(r1.is_some());
        assert!(r2.is_some());
    }
}
