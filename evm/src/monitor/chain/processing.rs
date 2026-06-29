//! Block processing: native and ERC20 payment detection.

use super::{ChainMonitor, PendingPayment, WatchKey, WatchedAddress};
use crate::error::EvmResult;
use crate::monitor::events::{MonitorEvent, PaymentDetected};
use crate::monitor::source::{BlockNotification, BlockSource, LogFilter};
use alloy::primitives::{Address, B256, U256};
use chrono::Utc;
use std::collections::HashMap;
use tracing::{debug, info, warn};

impl<S: BlockSource + 'static> ChainMonitor<S> {
    /// Process a new block.
    #[allow(clippy::cognitive_complexity)] // reorg check + watched-address scan is one logical unit
    pub(super) async fn process_block(&self, block: &BlockNotification) -> EvmResult<()> {
        let chain_id = self.chain_id();
        debug!(chain_id, block = block.number, "processing block");

        // Check for reorg
        if let Some(last_hash) = *self.last_block_hash.read().await
            && let Some(last_num) = *self.last_block.read().await
        {
            // If this block's parent doesn't match our last block, potential reorg
            if block.number == last_num + 1 && block.parent_hash != last_hash {
                warn!(
                    chain_id,
                    block = block.number,
                    expected_parent = %last_hash,
                    actual_parent = %block.parent_hash,
                    "potential reorg detected"
                );
                self.handle_reorg(last_num, last_hash, block).await?;
            }
        }

        // Get watched addresses (read lock)
        let has_native_watches;
        {
            let watched = self.watched.read().await;
            if watched.is_empty() {
                *self.last_block.write().await = Some(block.number);
                *self.last_block_hash.write().await = Some(block.hash);
                return Ok(());
            }

            has_native_watches = watched.keys().any(|(_, token)| token.is_none());

            // Check for native transfers (balance changes)
            if self.config.monitor_native {
                self.check_native_payments(&watched, block).await?;
            }

            // Check for ERC20 transfers
            if self.config.monitor_erc20 {
                self.check_erc20_payments(&watched, block).await?;
            }
        } // Release read lock

        // Update last known balances for native watches (requires write lock)
        if has_native_watches && self.config.monitor_native {
            self.update_watched_balances(block.number).await?;
        }

        *self.last_block.write().await = Some(block.number);
        *self.last_block_hash.write().await = Some(block.hash);

        Ok(())
    }

    /// Check for native currency payments.
    ///
    /// Uses a two-phase approach:
    /// 1. Poll balances to detect increases (lightweight)
    /// 2. Only when balance increases, fetch block transactions to get tx details
    async fn check_native_payments(
        &self,
        watched: &HashMap<WatchKey, WatchedAddress>,
        block: &BlockNotification,
    ) -> EvmResult<()> {
        // Collect native watched addresses and check for balance increases
        let mut addresses_with_increase: Vec<(Address, uuid::Uuid, U256)> = Vec::new();

        for ((address, token), watch) in watched.iter() {
            // Skip if watching for token (not native)
            if token.is_some() {
                continue;
            }

            let current_balance = self
                .source
                .get_balance(*address, Some(block.number))
                .await?;

            if current_balance > watch.last_known_balance {
                let increase = current_balance - watch.last_known_balance;
                addresses_with_increase.push((*address, watch.invoice_id, increase));
            }
        }

        // If no balance increases, nothing to do
        if addresses_with_increase.is_empty() {
            return Ok(());
        }

        // Fetch transactions for this block to find the actual transfers
        let addresses: Vec<Address> = addresses_with_increase.iter().map(|(a, _, _)| *a).collect();
        let transfers = self
            .source
            .find_native_transfers_to(block.number, &addresses)
            .await?;

        // Create a lookup map for quick access
        let invoice_map: HashMap<Address, uuid::Uuid> = addresses_with_increase
            .iter()
            .map(|(addr, invoice_id, _)| (*addr, *invoice_id))
            .collect();

        // Process each transfer found
        for transfer in transfers {
            let Some(&invoice_id) = invoice_map.get(&transfer.to) else {
                continue;
            };

            let event = PaymentDetected {
                chain_id: self.chain_id(),
                invoice_id,
                payment_address: transfer.to,
                amount: transfer.value,
                tx_hash: transfer.tx_hash,
                block_number: block.number,
                block_hash: block.hash,
                log_index: None,
                is_native: true,
                token_address: None,
                from_address: transfer.from,
                confirmations: 1,
                required_confirmations: self.config.required_confirmations,
                detected_at: Utc::now(),
            };

            info!(
                chain_id = self.chain_id(),
                invoice_id = %invoice_id,
                address = %transfer.to,
                amount = %transfer.value,
                tx = %transfer.tx_hash,
                from = %transfer.from,
                "native payment detected"
            );

            // Add to pending for confirmation tracking
            self.pending.write().await.insert(
                event.tx_hash,
                PendingPayment {
                    event: event.clone(),
                    last_check_block: block.number,
                },
            );

            let _ = self.event_tx.send(MonitorEvent::PaymentDetected(event));
        }

        Ok(())
    }

    /// Update last known balances for all watched native addresses.
    ///
    /// Called after payment checks to ensure we track the latest balance
    /// for detecting future payments.
    async fn update_watched_balances(&self, block_number: u64) -> EvmResult<()> {
        let mut watched = self.watched.write().await;

        for ((_, token), watch) in watched.iter_mut() {
            if token.is_some() {
                continue; // Skip ERC20
            }

            let balance = self
                .source
                .get_balance(watch.address, Some(block_number))
                .await?;
            watch.last_known_balance = balance;
        }

        Ok(())
    }

    /// Check for ERC20 token payments.
    async fn check_erc20_payments(
        &self,
        watched: &HashMap<WatchKey, WatchedAddress>,
        block: &BlockNotification,
    ) -> EvmResult<()> {
        // Collect unique addresses we're watching (for ERC20, token must be Some)
        let watch_addresses: Vec<Address> = watched
            .keys()
            .filter(|(_, token)| token.is_some())
            .map(|(addr, _)| *addr)
            .collect();
        if watch_addresses.is_empty() {
            return Ok(());
        }

        // Query Transfer logs for this block
        let filter = LogFilter::erc20_transfers_to(watch_addresses.clone())
            .with_block_range(block.number, block.number);

        let logs = self.source.get_logs(&filter).await?;

        for log in logs {
            // Decode Transfer event
            // topic0 = Transfer signature (already filtered)
            // topic1 = from address
            // topic2 = to address
            // data = amount

            if log.topics().len() < 3 {
                continue;
            }

            let to_address = Address::from_slice(&log.topics()[2].as_slice()[12..]);
            let token_address = log.address();

            // Check if this is an (address, token) pair we're watching
            let key = (to_address, Some(token_address));
            if let Some(watch) = watched.get(&key) {
                let from_address = Address::from_slice(&log.topics()[1].as_slice()[12..]);
                let amount = U256::from_be_slice(log.data().data.as_ref());

                let event = PaymentDetected {
                    chain_id: self.chain_id(),
                    invoice_id: watch.invoice_id,
                    payment_address: to_address,
                    amount,
                    tx_hash: log.transaction_hash.unwrap_or(B256::ZERO),
                    block_number: block.number,
                    block_hash: log.block_hash.unwrap_or(B256::ZERO),
                    log_index: log.log_index.map(|i| i as u32),
                    is_native: false,
                    token_address: Some(log.address()),
                    from_address,
                    confirmations: 1,
                    required_confirmations: self.config.required_confirmations,
                    detected_at: Utc::now(),
                };

                info!(
                    chain_id = self.chain_id(),
                    invoice_id = %watch.invoice_id,
                    %to_address,
                    token = %log.address(),
                    amount = %amount,
                    tx = %event.tx_hash,
                    "ERC20 payment detected"
                );

                // Add to pending
                self.pending.write().await.insert(
                    event.tx_hash,
                    PendingPayment {
                        event: event.clone(),
                        last_check_block: block.number,
                    },
                );

                let _ = self.event_tx.send(MonitorEvent::PaymentDetected(event));
            }
        }

        Ok(())
    }
}
