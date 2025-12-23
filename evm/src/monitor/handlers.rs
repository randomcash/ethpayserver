//! Event handlers for processing monitor events.

use super::events::MonitorEvent;
use crate::error::EvmResult;
use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Handler for monitor events.
#[async_trait]
pub trait EventHandler: Send + Sync {
    /// Handle a monitor event.
    async fn handle(&self, event: &MonitorEvent) -> EvmResult<()>;

    /// Get handler name for logging.
    fn name(&self) -> &str {
        "EventHandler"
    }
}

/// Function type for simple event handlers.
pub type EventHandlerFn =
    Arc<dyn Fn(&MonitorEvent) -> Pin<Box<dyn Future<Output = EvmResult<()>> + Send>> + Send + Sync>;

/// Wrapper to use a function as an EventHandler.
pub struct FnHandler {
    name: String,
    handler: EventHandlerFn,
}

impl FnHandler {
    /// Create a new function handler.
    pub fn new(name: impl Into<String>, handler: EventHandlerFn) -> Self {
        Self {
            name: name.into(),
            handler,
        }
    }
}

#[async_trait]
impl EventHandler for FnHandler {
    async fn handle(&self, event: &MonitorEvent) -> EvmResult<()> {
        (self.handler)(event).await
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Handler that logs all events at INFO level.
pub struct LoggingHandler;

impl LoggingHandler {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LoggingHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventHandler for LoggingHandler {
    async fn handle(&self, event: &MonitorEvent) -> EvmResult<()> {
        match event {
            MonitorEvent::PaymentDetected(p) => {
                tracing::info!(
                    chain_id = p.chain_id,
                    invoice_id = %p.invoice_id,
                    address = %p.payment_address,
                    amount = %p.amount,
                    tx = %p.tx_hash,
                    confirmations = p.confirmations,
                    "payment detected"
                );
            }
            MonitorEvent::PaymentConfirmed(p) => {
                tracing::info!(
                    chain_id = p.chain_id,
                    invoice_id = %p.invoice_id,
                    tx = %p.tx_hash,
                    confirmations = p.confirmations,
                    "payment confirmed"
                );
            }
            MonitorEvent::ReorgDetected(r) => {
                tracing::warn!(
                    chain_id = r.chain_id,
                    fork_block = r.fork_block,
                    depth = r.depth,
                    affected = r.affected_invoices.len(),
                    "reorg detected"
                );
            }
            MonitorEvent::MonitorStarted { chain_id } => {
                tracing::info!(chain_id, "monitor started");
            }
            MonitorEvent::MonitorStopped { chain_id } => {
                tracing::info!(chain_id, "monitor stopped");
            }
            MonitorEvent::MonitorError { chain_id, error } => {
                tracing::error!(chain_id, %error, "monitor error");
            }
            MonitorEvent::AddressWatched(w) => {
                tracing::info!(
                    chain_id = w.chain_id,
                    address = %w.address,
                    invoice_id = %w.invoice_id,
                    "address watched"
                );
            }
            MonitorEvent::AddressUnwatched(w) => {
                tracing::info!(
                    chain_id = w.chain_id,
                    address = %w.address,
                    invoice_id = ?w.invoice_id,
                    "address unwatched"
                );
            }
            MonitorEvent::StatusReport(s) => {
                tracing::info!(
                    chain_id = s.chain_id,
                    watched_count = s.watched_count,
                    current_block = s.current_block,
                    "status report"
                );
            }
        }
        Ok(())
    }

    fn name(&self) -> &str {
        "LoggingHandler"
    }
}

/// Handler that filters events by chain ID.
pub struct ChainFilter<H: EventHandler> {
    chain_ids: Vec<u64>,
    inner: H,
}

impl<H: EventHandler> ChainFilter<H> {
    pub fn new(chain_ids: Vec<u64>, inner: H) -> Self {
        Self { chain_ids, inner }
    }
}

#[async_trait]
impl<H: EventHandler> EventHandler for ChainFilter<H> {
    async fn handle(&self, event: &MonitorEvent) -> EvmResult<()> {
        let chain_id = match event {
            MonitorEvent::PaymentDetected(p) => p.chain_id,
            MonitorEvent::PaymentConfirmed(p) => p.chain_id,
            MonitorEvent::ReorgDetected(r) => r.chain_id,
            MonitorEvent::MonitorStarted { chain_id } => *chain_id,
            MonitorEvent::MonitorStopped { chain_id } => *chain_id,
            MonitorEvent::MonitorError { chain_id, .. } => *chain_id,
            MonitorEvent::AddressWatched(w) => w.chain_id,
            MonitorEvent::AddressUnwatched(w) => w.chain_id,
            MonitorEvent::StatusReport(s) => s.chain_id,
        };

        if self.chain_ids.contains(&chain_id) {
            self.inner.handle(event).await
        } else {
            Ok(())
        }
    }

    fn name(&self) -> &str {
        self.inner.name()
    }
}

/// Handler that only processes payment events (not lifecycle events).
pub struct PaymentOnlyFilter<H: EventHandler> {
    inner: H,
}

impl<H: EventHandler> PaymentOnlyFilter<H> {
    pub fn new(inner: H) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl<H: EventHandler> EventHandler for PaymentOnlyFilter<H> {
    async fn handle(&self, event: &MonitorEvent) -> EvmResult<()> {
        match event {
            MonitorEvent::PaymentDetected(_)
            | MonitorEvent::PaymentConfirmed(_)
            | MonitorEvent::ReorgDetected(_) => self.inner.handle(event).await,
            _ => Ok(()),
        }
    }

    fn name(&self) -> &str {
        self.inner.name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::events::PaymentDetected;
    use alloy::primitives::{Address, B256, U256};
    use chrono::Utc;

    struct CountingHandler {
        count: std::sync::atomic::AtomicUsize,
    }

    impl CountingHandler {
        fn new() -> Self {
            Self {
                count: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn count(&self) -> usize {
            self.count.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl EventHandler for CountingHandler {
        async fn handle(&self, _event: &MonitorEvent) -> EvmResult<()> {
            self.count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    fn make_payment_event(chain_id: u64) -> MonitorEvent {
        MonitorEvent::PaymentDetected(PaymentDetected {
            chain_id,
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
    async fn test_chain_filter() {
        let counter = CountingHandler::new();
        let filter = ChainFilter::new(vec![1, 137], counter);

        // Should process chain 1
        filter.handle(&make_payment_event(1)).await.unwrap();
        assert_eq!(filter.inner.count(), 1);

        // Should process chain 137
        filter.handle(&make_payment_event(137)).await.unwrap();
        assert_eq!(filter.inner.count(), 2);

        // Should skip chain 42161
        filter.handle(&make_payment_event(42161)).await.unwrap();
        assert_eq!(filter.inner.count(), 2);
    }

    #[tokio::test]
    async fn test_payment_only_filter() {
        let counter = CountingHandler::new();
        let filter = PaymentOnlyFilter::new(counter);

        // Should process payment event
        filter.handle(&make_payment_event(1)).await.unwrap();
        assert_eq!(filter.inner.count(), 1);

        // Should skip lifecycle event
        filter
            .handle(&MonitorEvent::MonitorStarted { chain_id: 1 })
            .await
            .unwrap();
        assert_eq!(filter.inner.count(), 1);
    }
}
