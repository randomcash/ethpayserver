//! In-memory event bridge using tokio broadcast channels.
//!
//! Useful for testing and single-process deployments where the monitor
//! runs in the same process as the API server.

use super::{EventBridge, EventStream};
use crate::error::EvmResult;
use crate::monitor::events::MonitorEvent;
use async_trait::async_trait;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

/// In-memory event bridge using tokio broadcast channels.
pub struct MemoryBridge {
    tx: broadcast::Sender<MonitorEvent>,
}

impl MemoryBridge {
    /// Create a new in-memory bridge.
    pub fn new() -> Self {
        Self::with_capacity(4096)
    }

    /// Create a new in-memory bridge with specified capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Get a raw broadcast sender for direct use.
    pub fn sender(&self) -> broadcast::Sender<MonitorEvent> {
        self.tx.clone()
    }
}

impl Default for MemoryBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventBridge for MemoryBridge {
    async fn publish(&self, event: &MonitorEvent) -> EvmResult<()> {
        // Ignore send errors (no receivers is fine)
        let _ = self.tx.send(event.clone());
        Ok(())
    }

    async fn subscribe(&self) -> EvmResult<EventStream> {
        let rx = self.tx.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(|result| result.ok());
        Ok(Box::pin(stream))
    }

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
    use crate::monitor::events::PaymentDetected;
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

    #[tokio::test]
    async fn test_memory_bridge_pubsub() {
        let bridge = MemoryBridge::new();

        // Subscribe first
        let mut stream = bridge.subscribe().await.unwrap();

        // Publish event
        let event = make_event();
        bridge.publish(&event).await.unwrap();

        // Receive event
        let received = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            stream.next(),
        )
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
