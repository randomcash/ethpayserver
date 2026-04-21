//! Chain monitor - monitors a single EVM chain for payments.

use super::events::{MonitorEvent, PaymentConfirmed, PaymentDetected, ReorgDetected};
use super::source::{BlockNotification, BlockSource, ChainHealth, LogFilter, SourceStatus};
use crate::error::{EvmError, EvmResult};
use crate::network::ChainConfig;
use alloy::primitives::{Address, B256, U256};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast, mpsc};
use tokio_stream::StreamExt;
use tracing::{debug, error, info, warn};

/// Configuration for the chain monitor.
#[derive(Debug, Clone)]
pub struct ChainMonitorConfig {
    /// Number of confirmations required.
    pub required_confirmations: u64,
    /// Maximum blocks to scan per iteration.
    pub max_blocks_per_scan: u64,
    /// How often to check pending payments (seconds).
    pub confirmation_check_interval_secs: u64,
    /// Whether to detect native (ETH) transfers.
    pub monitor_native: bool,
    /// Whether to detect ERC20 transfers.
    pub monitor_erc20: bool,
}

impl Default for ChainMonitorConfig {
    fn default() -> Self {
        Self {
            required_confirmations: 12,
            max_blocks_per_scan: 100,
            confirmation_check_interval_secs: 30,
            monitor_native: true,
            monitor_erc20: true,
        }
    }
}

impl ChainMonitorConfig {
    /// Create config from chain defaults.
    pub fn from_chain(chain: &ChainConfig) -> Self {
        Self {
            required_confirmations: chain.confirmations_required as u64,
            ..Default::default()
        }
    }
}

/// An address being watched for payments.
#[derive(Debug, Clone)]
pub struct WatchedAddress {
    /// The address to watch.
    pub address: Address,
    /// Invoice ID this address is for.
    pub invoice_id: uuid::Uuid,
    /// Expected amount (if known).
    pub expected_amount: Option<U256>,
    /// Token contract (None = native).
    pub token_contract: Option<Address>,
    /// When watching started.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Previous known balance (for native).
    pub last_known_balance: U256,
}

/// State of a pending payment awaiting confirmations.
#[derive(Debug, Clone)]
struct PendingPayment {
    event: PaymentDetected,
    #[allow(dead_code)]
    last_check_block: u64,
}

/// Key for watched address map: (address, token_contract).
/// token_contract is None for native asset, Some(addr) for ERC20.
pub type WatchKey = (Address, Option<Address>);

/// Monitor for a single EVM chain.
pub struct ChainMonitor<S: BlockSource> {
    /// Chain configuration.
    chain_config: &'static ChainConfig,
    /// Monitor configuration.
    config: ChainMonitorConfig,
    /// Block source.
    source: Arc<S>,
    /// Addresses being watched, keyed by (address, token_contract).
    /// This allows the same address to be watched for different tokens.
    watched: RwLock<HashMap<WatchKey, WatchedAddress>>,
    /// Payments pending confirmation.
    pending: RwLock<HashMap<B256, PendingPayment>>,
    /// Last processed block.
    last_block: RwLock<Option<u64>>,
    /// Block hash at last processed block (for reorg detection).
    last_block_hash: RwLock<Option<B256>>,
    /// Event sender.
    event_tx: broadcast::Sender<MonitorEvent>,
    /// Shutdown signal.
    shutdown_tx: mpsc::Sender<()>,
    shutdown_rx: RwLock<Option<mpsc::Receiver<()>>>,
}

impl<S: BlockSource + 'static> ChainMonitor<S> {
    /// Create a new chain monitor.
    pub fn new(chain_config: &'static ChainConfig, source: S, config: ChainMonitorConfig) -> Self {
        let (event_tx, _) = broadcast::channel(1024);
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

        Self {
            chain_config,
            config,
            source: Arc::new(source),
            watched: RwLock::new(HashMap::new()),
            pending: RwLock::new(HashMap::new()),
            last_block: RwLock::new(None),
            last_block_hash: RwLock::new(None),
            event_tx,
            shutdown_tx,
            shutdown_rx: RwLock::new(Some(shutdown_rx)),
        }
    }

    /// Get the chain ID.
    pub fn chain_id(&self) -> u64 {
        self.chain_config.chain_id
    }

    /// Get chain name.
    pub fn chain_name(&self) -> &str {
        self.chain_config.name
    }

    /// Subscribe to monitor events.
    pub fn subscribe(&self) -> broadcast::Receiver<MonitorEvent> {
        self.event_tx.subscribe()
    }

    /// Add an address to watch.
    pub async fn watch(&self, watched: WatchedAddress) {
        let key = (watched.address, watched.token_contract);
        let address = watched.address;
        let token = watched.token_contract;
        self.watched.write().await.insert(key, watched);
        debug!(
            chain_id = self.chain_id(),
            %address,
            token = ?token,
            "watching address"
        );
    }

    /// Remove an address from watch list.
    /// token_contract should be None for native, Some(addr) for ERC20.
    pub async fn unwatch(
        &self,
        address: &Address,
        token_contract: Option<Address>,
    ) -> Option<WatchedAddress> {
        let key = (*address, token_contract);
        let removed = self.watched.write().await.remove(&key);
        if removed.is_some() {
            debug!(
                chain_id = self.chain_id(),
                %address,
                token = ?token_contract,
                "unwatched address"
            );
        }
        removed
    }

    /// Get all watched addresses.
    pub async fn watched_addresses(&self) -> Vec<WatchedAddress> {
        self.watched.read().await.values().cloned().collect()
    }

    /// Get the current block number.
    pub async fn current_block(&self) -> EvmResult<u64> {
        self.source.get_block_number().await
    }

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

    /// Process a new block.
    #[allow(clippy::cognitive_complexity)] // reorg check + watched-address scan is one logical unit
    async fn process_block(&self, block: &BlockNotification) -> EvmResult<()> {
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

    /// Check confirmation status of pending payments.
    async fn check_confirmations(&self) -> EvmResult<()> {
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
    async fn handle_reorg(
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

impl<S: BlockSource> std::fmt::Debug for ChainMonitor<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChainMonitor")
            .field("chain", &self.chain_config.name)
            .field("chain_id", &self.chain_config.chain_id)
            .finish()
    }
}
