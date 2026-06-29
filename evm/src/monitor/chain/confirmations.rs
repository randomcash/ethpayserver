//! Confirmation tracking and reorg handling.

use super::ChainMonitor;
use crate::error::EvmResult;
use crate::monitor::events::{MonitorEvent, PaymentConfirmed, ReorgDetected};
use crate::monitor::source::{BlockNotification, BlockSource};
use alloy::primitives::B256;
use chrono::Utc;
use tracing::{info, warn};

impl<S: BlockSource + 'static> ChainMonitor<S> {
    /// Check confirmation status of pending payments.
    pub(super) async fn check_confirmations(&self) -> EvmResult<()> {
        let current_block = self.source.get_block_number().await?;
        let mut confirmed = Vec::new();

        {
            let mut pending = self.pending.write().await;

            for (tx_hash, payment) in pending.iter_mut() {
                let confirmations = current_block.saturating_sub(payment.event.block_number) + 1;
                payment.event.confirmations = confirmations;

                if confirmations >= payment.event.required_confirmations {
                    confirmed.push(*tx_hash);

                    let confirm_event = PaymentConfirmed {
                        chain_id: payment.event.chain_id,
                        invoice_id: payment.event.invoice_id,
                        payment_address: payment.event.payment_address,
                        amount: payment.event.amount,
                        tx_hash: payment.event.tx_hash,
                        block_number: payment.event.block_number,
                        confirmations,
                        confirmed_at: Utc::now(),
                    };

                    info!(
                        chain_id = self.chain_id(),
                        invoice_id = %payment.event.invoice_id,
                        tx = %tx_hash,
                        confirmations,
                        "payment confirmed"
                    );

                    let _ = self
                        .event_tx
                        .send(MonitorEvent::PaymentConfirmed(confirm_event));
                }
            }

            // Remove confirmed payments
            for tx_hash in &confirmed {
                pending.remove(tx_hash);
            }
        }

        Ok(())
    }

    /// Handle a detected chain reorganization.
    pub(super) async fn handle_reorg(
        &self,
        fork_block: u64,
        old_hash: B256,
        new_block: &BlockNotification,
    ) -> EvmResult<()> {
        // Find affected invoices
        let pending = self.pending.read().await;
        let affected: Vec<uuid::Uuid> = pending
            .values()
            .filter(|p| p.event.block_number >= fork_block)
            .map(|p| p.event.invoice_id)
            .collect();

        let depth = new_block.number.saturating_sub(fork_block) + 1;

        let event = ReorgDetected {
            chain_id: self.chain_id(),
            fork_block,
            old_hash,
            new_hash: new_block.hash,
            depth,
            affected_invoices: affected.clone(),
            detected_at: Utc::now(),
        };

        warn!(
            chain_id = self.chain_id(),
            fork_block,
            depth,
            affected_count = affected.len(),
            "reorg detected"
        );

        let _ = self.event_tx.send(MonitorEvent::ReorgDetected(event));

        // Re-check affected payments
        // In a real implementation, we'd re-validate these transactions

        Ok(())
    }
}
