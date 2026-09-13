//! Confirmation tracking and reorg handling.

use super::ChainMonitor;
use crate::error::EvmResult;
use crate::monitor::events::{MonitorEvent, PaymentConfirmed, ReorgDetected};
use crate::monitor::source::{BlockNotification, BlockSource, LogFilter};
use alloy::primitives::{Address, B256};
use chrono::Utc;
use tracing::{info, warn};

impl<S: BlockSource + 'static> ChainMonitor<S> {
    /// Check confirmation status of pending payments.
    pub(super) async fn check_confirmations(&self) -> EvmResult<()> {
        let current_block = self.source.get_block_number().await?;
        let mut confirmed: Vec<(B256, i32)> = Vec::new();

        {
            let mut pending = self.pending.write().await;

            for ((tx_hash, tx_index), payment) in pending.iter_mut() {
                let confirmations = current_block.saturating_sub(payment.event.block_number) + 1;
                payment.event.confirmations = confirmations;

                if confirmations >= payment.event.required_confirmations {
                    confirmed.push((*tx_hash, *tx_index));

                    let confirm_event = PaymentConfirmed {
                        chain_id: payment.event.chain_id,
                        invoice_id: payment.event.invoice_id,
                        payment_address: payment.event.payment_address,
                        amount: payment.event.amount,
                        tx_hash: payment.event.tx_hash,
                        tx_index: *tx_index,
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
            for key in &confirmed {
                pending.remove(key);
            }
        }

        Ok(())
    }

    /// Handle a detected chain reorganization.
    ///
    /// `affected_invoices` is a best-effort hint drawn from payments this
    /// monitor still has in memory: it is empty right after a restart and
    /// never includes a payment that has already confirmed and dropped out
    /// of `pending`. The database, not this event, is the source of truth
    /// for which payments a reorg actually touches — see the server's
    /// reorg handler, which finds the real candidate set there.
    ///
    /// What this monitor *can* answer authoritatively is whether a specific
    /// transaction is still on chain: `survived_tx_hashes` re-scans
    /// `[fork_block, new_block.number]` for currently watched addresses, so
    /// a transaction the reorg merely relocated to a different block is
    /// reported as still present rather than assumed gone. Retracting one of
    /// those would be the opposite error — un-paying an invoice that is
    /// still genuinely paid.
    pub(super) async fn handle_reorg(
        &self,
        fork_block: u64,
        old_hash: B256,
        new_block: &BlockNotification,
    ) -> EvmResult<()> {
        // Find affected invoices (best-effort hint, see doc comment above)
        let pending = self.pending.read().await;
        let affected: Vec<uuid::Uuid> = pending
            .values()
            .filter(|p| p.event.block_number >= fork_block)
            .map(|p| p.event.invoice_id)
            .collect();
        drop(pending);

        let depth = new_block.number.saturating_sub(fork_block) + 1;

        let survived_tx_hashes = match self
            .find_survived_tx_hashes(fork_block, new_block.number)
            .await
        {
            Ok(hashes) => hashes,
            Err(e) => {
                warn!(
                    chain_id = self.chain_id(),
                    fork_block,
                    error = %e,
                    "failed to re-validate reorg against the chain; nothing reported as survived"
                );
                Vec::new()
            }
        };

        let event = ReorgDetected {
            chain_id: self.chain_id(),
            fork_block,
            old_hash,
            new_hash: new_block.hash,
            depth,
            affected_invoices: affected.clone(),
            survived_tx_hashes,
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

        Ok(())
    }

    /// Re-scan `[from, to]` for transfers to currently watched addresses, to
    /// find transactions a reorg relocated to a different block rather than
    /// dropped entirely.
    ///
    /// Watched addresses are restored from persistence on monitor restart, so
    /// this still works after one — unlike `pending`, which starts empty. It
    /// misses a payment whose address has since been unwatched (e.g. an
    /// expired, cleaned-up invoice), which is a known limitation: confirming
    /// survival for those would require asking the chain about a specific
    /// historical transaction hash, which no `BlockSource` method does today.
    async fn find_survived_tx_hashes(&self, from: u64, to: u64) -> EvmResult<Vec<B256>> {
        let watched = self.watched.read().await;
        if watched.is_empty() {
            return Ok(Vec::new());
        }
        let native_addresses: Vec<Address> = watched
            .keys()
            .filter(|(_, token)| token.is_none())
            .map(|(address, _)| *address)
            .collect();
        let erc20_addresses: Vec<Address> = watched
            .keys()
            .filter(|(_, token)| token.is_some())
            .map(|(address, _)| *address)
            .collect();
        drop(watched);

        // Bounded so a very deep reorg cannot turn this into an unbounded
        // number of RPC calls; reuses the same knob block processing uses to
        // limit how much history it scans.
        let scan_from = from.max(to.saturating_sub(self.config.max_blocks_per_scan));

        let mut survived = Vec::new();

        if !native_addresses.is_empty() {
            for block_number in scan_from..=to {
                let transfers = self
                    .source
                    .find_native_transfers_to(block_number, &native_addresses)
                    .await?;
                survived.extend(transfers.into_iter().map(|t| t.tx_hash));
            }
        }

        if !erc20_addresses.is_empty() {
            let filter =
                LogFilter::erc20_transfers_to(erc20_addresses).with_block_range(scan_from, to);
            let logs = self.source.get_logs(&filter).await?;
            survived.extend(logs.into_iter().filter_map(|l| l.transaction_hash));
        }

        Ok(survived)
    }
}
