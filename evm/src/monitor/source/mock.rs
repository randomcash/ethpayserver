//! Mock block source for integration testing.
//!
//! Allows injecting synthetic blocks, balances, native transfers, and ERC20 logs
//! to test the payment detection pipeline without real RPC connections.
//!
//! Uses shared interior state (`Arc`) so `Clone` copies share the same data.
//! This lets a test retain a handle for injection while `ChainMonitor` owns another.

use super::{BlockNotification, BlockSource, BlockStream, LogFilter, NativeTransfer, SourceStatus};
use crate::error::EvmResult;
use alloy::primitives::{Address, B256, U256};
use alloy::rpc::types::Block;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{RwLock, broadcast};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

/// Shared inner state for the mock source.
struct Inner {
    chain_id: u64,
    current_block: AtomicU64,
    balances: RwLock<HashMap<Address, U256>>,
    native_transfers: RwLock<HashMap<u64, Vec<NativeTransfer>>>,
    logs: RwLock<HashMap<u64, Vec<alloy::rpc::types::Log>>>,
    block_tx: broadcast::Sender<EvmResult<BlockNotification>>,
}

/// A mock block source for testing payment detection.
///
/// Cloning produces a handle to the **same** underlying state, so data injected
/// through one handle is visible from the other. This is essential because
/// `ChainMonitor::new` takes ownership of the source while the test code
/// needs to inject blocks and balances after the monitor starts.
///
/// # Usage
/// ```ignore
/// let source = MockBlockSource::new(1);
/// let test_handle = source.clone(); // same state
/// let monitor = ChainMonitor::new(chain_config, source, config);
/// // test_handle.set_balance(...), test_handle.push_block(...)
/// ```
#[derive(Clone)]
pub struct MockBlockSource {
    inner: Arc<Inner>,
}

impl MockBlockSource {
    /// Create a new mock source for the given chain ID.
    pub fn new(chain_id: u64) -> Self {
        let (block_tx, _) = broadcast::channel(256);
        Self {
            inner: Arc::new(Inner {
                chain_id,
                current_block: AtomicU64::new(0),
                balances: RwLock::new(HashMap::new()),
                native_transfers: RwLock::new(HashMap::new()),
                logs: RwLock::new(HashMap::new()),
                block_tx,
            }),
        }
    }

    /// Push a block notification to all subscribers.
    pub fn push_block(&self, block: BlockNotification) {
        self.inner
            .current_block
            .store(block.number, Ordering::SeqCst);
        let _ = self.inner.block_tx.send(Ok(block));
    }

    /// Set the balance for an address.
    pub async fn set_balance(&self, address: Address, balance: U256) {
        self.inner.balances.write().await.insert(address, balance);
    }

    /// Add a native transfer to a specific block.
    pub async fn add_native_transfer(&self, block_number: u64, transfer: NativeTransfer) {
        self.inner
            .native_transfers
            .write()
            .await
            .entry(block_number)
            .or_default()
            .push(transfer);
    }

    /// Add an ERC20 Transfer log to a specific block.
    pub async fn add_log(&self, block_number: u64, log: alloy::rpc::types::Log) {
        self.inner
            .logs
            .write()
            .await
            .entry(block_number)
            .or_default()
            .push(log);
    }

    /// Set the current block number without pushing a notification.
    pub fn set_block_number(&self, number: u64) {
        self.inner.current_block.store(number, Ordering::SeqCst);
    }
}

#[async_trait]
impl BlockSource for MockBlockSource {
    fn chain_id(&self) -> u64 {
        self.inner.chain_id
    }

    fn status(&self) -> SourceStatus {
        SourceStatus::Connected
    }

    async fn subscribe_blocks(&self) -> EvmResult<BlockStream> {
        let rx = self.inner.block_tx.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(|result| match result {
            Ok(Ok(block)) => Some(Ok(block)),
            Ok(Err(e)) => Some(Err(e)),
            Err(_) => None, // Lagged receiver, skip
        });
        Ok(Box::pin(stream))
    }

    async fn get_logs(&self, filter: &LogFilter) -> EvmResult<Vec<alloy::rpc::types::Log>> {
        let logs = self.inner.logs.read().await;
        let mut result = Vec::new();

        let from = filter.from_block.unwrap_or(0);
        let to = filter.to_block.unwrap_or(u64::MAX);

        for block_num in from..=to {
            if let Some(block_logs) = logs.get(&block_num) {
                for log in block_logs {
                    if !filter.topics.is_empty() {
                        let matches = filter.topics.iter().enumerate().all(|(i, topic_filter)| {
                            match topic_filter {
                                Some(expected) => {
                                    log.topics().get(i) == Some(expected)
                                }
                                None => true,
                            }
                        });
                        if !matches {
                            continue;
                        }
                    }
                    result.push(log.clone());
                }
            }
        }

        Ok(result)
    }

    async fn get_balance(&self, address: Address, _block: Option<u64>) -> EvmResult<U256> {
        let balances = self.inner.balances.read().await;
        Ok(balances.get(&address).copied().unwrap_or(U256::ZERO))
    }

    async fn get_block_number(&self) -> EvmResult<u64> {
        Ok(self.inner.current_block.load(Ordering::SeqCst))
    }

    async fn get_block(&self, _number: u64) -> EvmResult<Option<Block>> {
        Ok(None)
    }

    async fn find_native_transfers_to(
        &self,
        block_number: u64,
        addresses: &[Address],
    ) -> EvmResult<Vec<NativeTransfer>> {
        let transfers = self.inner.native_transfers.read().await;
        Ok(transfers
            .get(&block_number)
            .map(|txs| {
                txs.iter()
                    .filter(|tx| addresses.contains(&tx.to))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn is_healthy(&self) -> bool {
        true
    }
}

/// Create a synthetic block notification with random hashes.
pub fn make_block(number: u64) -> BlockNotification {
    make_block_with_parent(number, B256::random(), B256::random())
}

/// Create a synthetic block notification with specific hashes.
pub fn make_block_with_parent(number: u64, hash: B256, parent_hash: B256) -> BlockNotification {
    BlockNotification {
        number,
        hash,
        parent_hash,
        timestamp: 1700000000 + number * 12,
    }
}

/// Create a synthetic native transfer.
pub fn make_native_transfer(
    from: Address,
    to: Address,
    value: U256,
    tx_hash: B256,
) -> NativeTransfer {
    NativeTransfer {
        tx_hash,
        from,
        to,
        value,
        tx_index: 0,
    }
}

/// Build an ERC20 Transfer log for testing.
///
/// Encodes: `Transfer(from, to, amount)` at the given token contract address.
pub fn make_erc20_transfer_log(
    token_contract: Address,
    from: Address,
    to: Address,
    amount: U256,
    block_number: u64,
    tx_hash: B256,
    log_index: u64,
) -> alloy::rpc::types::Log {
    use alloy::primitives::LogData;

    // topic0 = keccak256("Transfer(address,address,uint256)")
    let transfer_topic: B256 = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
        .parse()
        .expect("valid transfer topic");

    // Pad address (20 bytes) to 32 bytes, left-aligned with zeros
    let mut from_bytes = [0u8; 32];
    from_bytes[12..].copy_from_slice(from.as_slice());
    let from_topic = B256::from(from_bytes);

    let mut to_bytes = [0u8; 32];
    to_bytes[12..].copy_from_slice(to.as_slice());
    let to_topic = B256::from(to_bytes);
    let amount_bytes: [u8; 32] = amount.to_be_bytes();

    let log_data = LogData::new(
        vec![transfer_topic, from_topic, to_topic],
        amount_bytes.to_vec().into(),
    )
    .expect("valid log data");

    alloy::rpc::types::Log {
        inner: alloy::primitives::Log {
            address: token_contract,
            data: log_data,
        },
        block_hash: Some(B256::random()),
        block_number: Some(block_number),
        block_timestamp: Some(1700000000 + block_number * 12),
        transaction_hash: Some(tx_hash),
        transaction_index: Some(0),
        log_index: Some(log_index),
        removed: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_source_basics() {
        let source = MockBlockSource::new(1);
        assert_eq!(source.chain_id(), 1);
        assert_eq!(source.status(), SourceStatus::Connected);
        assert!(source.is_healthy().await);
        assert_eq!(source.get_block_number().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_mock_source_clone_shares_state() {
        let source = MockBlockSource::new(1);
        let handle = source.clone();
        let addr = Address::random();

        source.set_balance(addr, U256::from(42)).await;
        assert_eq!(
            handle.get_balance(addr, None).await.unwrap(),
            U256::from(42)
        );
    }

    #[tokio::test]
    async fn test_mock_source_balance() {
        let source = MockBlockSource::new(1);
        let addr = Address::random();

        assert_eq!(source.get_balance(addr, None).await.unwrap(), U256::ZERO);
        source.set_balance(addr, U256::from(1_000_000)).await;
        assert_eq!(
            source.get_balance(addr, None).await.unwrap(),
            U256::from(1_000_000)
        );
    }

    #[tokio::test]
    async fn test_mock_source_native_transfers() {
        let source = MockBlockSource::new(1);
        let to_addr = Address::random();
        let transfer = make_native_transfer(
            Address::random(),
            to_addr,
            U256::from(1_000),
            B256::random(),
        );

        source.add_native_transfer(100, transfer).await;

        let found = source
            .find_native_transfers_to(100, &[to_addr])
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].to, to_addr);

        let found = source
            .find_native_transfers_to(100, &[Address::random()])
            .await
            .unwrap();
        assert!(found.is_empty());
    }

    #[tokio::test]
    async fn test_mock_source_block_stream() {
        let source = MockBlockSource::new(1);
        let mut stream = source.subscribe_blocks().await.unwrap();

        let block = make_block(42);
        source.push_block(block);

        let received = tokio::time::timeout(std::time::Duration::from_millis(100), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        assert_eq!(received.number, 42);
        assert_eq!(source.get_block_number().await.unwrap(), 42);
    }
}
