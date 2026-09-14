//! Monitor lifecycle: health reporting, start/stop event loop.

use super::ChainMonitor;
use crate::error::{EvmError, EvmResult};
use crate::monitor::events::MonitorEvent;
use crate::monitor::source::{BlockSource, BlockStream, ChainHealth, SourceStatus};
use std::sync::Arc;
use tokio_stream::StreamExt;
use tracing::{error, info, warn};

impl<S: BlockSource + 'static> ChainMonitor<S> {
    /// Get health information for this chain.
    pub async fn get_health(&self) -> ChainHealth {
        let status = self.source.status();
        let current_block = self.source.get_block_number().await.ok();
        let last_processed_block = *self.last_block.read().await;
        let watched_addresses = self.watched.read().await.len();

        // Consider healthy if connected and not lagging more than 10 blocks
        let is_healthy = status == SourceStatus::Connected
            && match (current_block, last_processed_block) {
                (Some(current), Some(last)) => current.saturating_sub(last) <= 10,
                (Some(_), None) => true, // Just started, not yet processed
                _ => false,
            };

        ChainHealth {
            chain_id: self.chain_id(),
            chain_name: self.chain_name().to_string(),
            status,
            current_block,
            last_processed_block,
            watched_addresses,
            is_healthy,
        }
    }

    /// Start the monitor.
    #[allow(clippy::cognitive_complexity)] // tokio::select! event loop with multiple branches
    pub async fn start(self: Arc<Self>) -> EvmResult<()> {
        let chain_id = self.chain_id();
        info!(
            chain_id,
            chain = self.chain_name(),
            "starting chain monitor"
        );

        // Get shutdown receiver
        let mut shutdown_rx = self
            .shutdown_rx
            .write()
            .await
            .take()
            .ok_or_else(|| EvmError::Monitor("monitor already started".to_string()))?;

        // Subscribe to blocks
        let mut block_stream = self.source.subscribe_blocks().await?;

        // Emit start event
        let _ = self
            .event_tx
            .send(MonitorEvent::MonitorStarted { chain_id });

        // Confirmation check timer
        let mut confirm_interval = tokio::time::interval(tokio::time::Duration::from_secs(
            self.config.confirmation_check_interval_secs,
        ));

        loop {
            tokio::select! {
                // Shutdown signal
                _ = shutdown_rx.recv() => {
                    info!(chain_id, "chain monitor shutting down");
                    let _ = self.event_tx.send(MonitorEvent::MonitorStopped { chain_id });
                    break;
                }

                // New block
                Some(block_result) = block_stream.next() => {
                    match block_result {
                        Ok(block) => {
                            if let Err(e) = self.process_block(&block).await {
                                error!(chain_id, error = %e, "error processing block");
                                let _ = self.event_tx.send(MonitorEvent::MonitorError {
                                    chain_id,
                                    error: e.to_string(),
                                });
                            }
                        }
                        Err(e) => {
                            error!(chain_id, error = %e, "block stream error");
                        }
                    }
                }

                // Confirmation check timer
                _ = confirm_interval.tick() => {
                    if let Err(e) = self.check_confirmations().await {
                        warn!(chain_id, error = %e, "error checking confirmations");
                    }

                    if let Err(e) = self.resubscribe_if_stalled(&mut block_stream).await {
                        warn!(chain_id, error = %e, "failed to resubscribe stalled block stream");
                    }
                }
            }
        }

        Ok(())
    }

    /// Stop the monitor.
    pub async fn stop(&self) -> EvmResult<()> {
        let _ = self.shutdown_tx.send(()).await;
        Ok(())
    }

    /// Notice an unhealthy chain - stalled or disconnected - and reconnect it.
    ///
    /// Two failure shapes land here. A dropped or half-open WebSocket doesn't
    /// always deliver a close frame - sometimes the subscription just stops
    /// yielding blocks forever, with no error to log and nothing to select!
    /// on, leaving the RPC reachable (health checks that ask it directly
    /// still succeed) while the block stream is dead. Or the connection
    /// itself goes down (`Disconnected`/`Failed`), and nothing retries it:
    /// `subscribe_blocks` is otherwise only ever called once, at startup.
    /// Both cases are indistinguishable from a healthy chain unless something
    /// checks, so this reuses `is_healthy` - which already covers "not
    /// connected" and "connected but lagging" - rather than invent a second
    /// definition of "unhealthy". This runs on the confirmation-check timer
    /// because that's the one thing already on a clock here.
    async fn resubscribe_if_stalled(&self, block_stream: &mut BlockStream) -> EvmResult<()> {
        let health = self.get_health().await;
        if health.is_healthy {
            return Ok(());
        }

        error!(
            chain_id = self.chain_id(),
            status = ?health.status,
            current_block = ?health.current_block,
            last_processed_block = ?health.last_processed_block,
            "chain unhealthy; resubscribing"
        );

        *block_stream = self.source.subscribe_blocks().await?;
        Ok(())
    }
}
