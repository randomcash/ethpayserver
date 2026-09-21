//! In-memory event bridge using a durable outbox behind a mutex.
//!
//! Useful for testing and single-process deployments where the monitor
//! runs in the same process as the API server.
//!
//! Supports bidirectional communication:
//! - Events flow from monitor to API server, through a durable outbox that
//!   survives a consumer dropping its stream and resubscribing (unlike the
//!   commands side, which stays a plain broadcast: `watch_retry` already
//!   covers a lost command).
//! - Commands flow from API server to monitor

use super::{CommandStream, DurableEventStream, EventBridge, EventCursor, EventEnvelope};
use crate::error::EvmResult;
use crate::monitor::events::{MonitorCommand, MonitorEvent};
use async_stream::stream;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use tokio::sync::{Notify, broadcast};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

/// The event outbox: every published envelope, in publish order.
///
/// A `Vec` behind a `Mutex` rather than a broadcast channel because the
/// whole point is that a consumer which was not subscribed when an event
/// was published can still see it later - a broadcast channel drops exactly
/// that message, which is the bug this bridge exists to not reproduce.
/// `entries[i].cursor.seq == i` always, so resuming from a cursor is a plain
/// slice index.
struct Outbox {
    epoch: i64,
    entries: Vec<EventEnvelope>,
}

/// In-memory event bridge with a durable event outbox.
pub struct MemoryBridge {
    outbox: Arc<Mutex<Outbox>>,
    /// Woken on every publish so a live `subscribe_from` tail notices new
    /// entries without polling.
    notify: Arc<Notify>,
    /// Commands channel (API server -> monitor). Fire-and-forget is fine
    /// here: `watch_retry` re-drives a lost command within 30 seconds.
    commands_tx: broadcast::Sender<MonitorCommand>,
}

impl MemoryBridge {
    /// Create a new in-memory bridge.
    pub fn new() -> Self {
        Self::with_capacity(4096)
    }

    /// Create a new in-memory bridge with specified capacity for the
    /// commands channel. The event outbox is unbounded: durability is the
    /// point, so there is nothing safe to drop from it here.
    pub fn with_capacity(capacity: usize) -> Self {
        let (commands_tx, _) = broadcast::channel(capacity);
        Self {
            outbox: Arc::new(Mutex::new(Outbox {
                epoch: 1,
                entries: Vec::new(),
            })),
            notify: Arc::new(Notify::new()),
            commands_tx,
        }
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
        {
            let mut outbox = self.outbox.lock().expect("outbox mutex poisoned");
            let seq = outbox.entries.len() as i64;
            let cursor = EventCursor {
                epoch: outbox.epoch,
                seq,
                block_height: event.block_height() as i64,
            };
            outbox.entries.push(EventEnvelope {
                chain_id: event.chain_id(),
                cursor,
                event: event.clone(),
            });
        }
        self.notify.notify_waiters();
        Ok(())
    }

    async fn subscribe_from(&self, from: Option<EventCursor>) -> EvmResult<DurableEventStream> {
        let outbox = Arc::clone(&self.outbox);
        let notify = Arc::clone(&self.notify);
        let mut next_index = from.map(|c| (c.seq + 1).max(0) as usize).unwrap_or(0);

        let s = stream! {
            loop {
                // Register interest before checking, so a publish landing
                // between the check and the await is never missed.
                let notified = notify.notified();

                let batch: Vec<EventEnvelope> = {
                    let guard = outbox.lock().expect("outbox mutex poisoned");
                    guard.entries.get(next_index..).map(<[_]>::to_vec).unwrap_or_default()
                };

                if batch.is_empty() {
                    notified.await;
                    continue;
                }

                for envelope in batch {
                    next_index += 1;
                    yield envelope;
                }
            }
        };
        Ok(Box::pin(s))
    }

    async fn current_epoch(&self) -> EvmResult<i64> {
        Ok(self.outbox.lock().expect("outbox mutex poisoned").epoch)
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
    async fn test_memory_bridge_events_durable() {
        let bridge = MemoryBridge::new();

        // Subscribe first
        let mut stream = bridge.subscribe_from(None).await.unwrap();

        // Publish event
        let event = make_event();
        bridge.publish(&event).await.unwrap();

        // Receive event
        let received = tokio::time::timeout(std::time::Duration::from_millis(100), stream.next())
            .await
            .unwrap()
            .unwrap();

        match received.event {
            MonitorEvent::PaymentDetected(p) => {
                assert_eq!(p.chain_id, 1);
            }
            _ => panic!("unexpected event type"),
        }
        assert_eq!(received.cursor.seq, 0);
    }

    #[tokio::test]
    async fn test_memory_bridge_events_survive_a_late_subscriber() {
        let bridge = MemoryBridge::new();

        // Nobody is subscribed yet when this publishes - the point of a
        // durable outbox is that this is not lost, unlike plain pub/sub.
        bridge.publish(&make_event()).await.unwrap();

        let mut stream = bridge.subscribe_from(None).await.unwrap();
        let received = tokio::time::timeout(std::time::Duration::from_millis(100), stream.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received.cursor.seq, 0);
    }

    #[tokio::test]
    async fn test_memory_bridge_resumes_after_a_cursor() {
        let bridge = MemoryBridge::new();

        bridge.publish(&make_event()).await.unwrap(); // seq 0
        bridge.publish(&make_event()).await.unwrap(); // seq 1
        bridge.publish(&make_event()).await.unwrap(); // seq 2

        let cursor = EventCursor {
            epoch: bridge.current_epoch().await.unwrap(),
            seq: 0,
            block_height: 0,
        };
        let mut stream = bridge.subscribe_from(Some(cursor)).await.unwrap();

        let first = tokio::time::timeout(std::time::Duration::from_millis(100), stream.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.cursor.seq, 1);

        let second = tokio::time::timeout(std::time::Duration::from_millis(100), stream.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.cursor.seq, 2);
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

        let mut stream1 = bridge.subscribe_from(None).await.unwrap();
        let mut stream2 = bridge.subscribe_from(None).await.unwrap();

        let event = make_event();
        bridge.publish(&event).await.unwrap();

        // Both should receive
        let r1 = stream1.next().await;
        let r2 = stream2.next().await;

        assert!(r1.is_some());
        assert!(r2.is_some());
    }
}
