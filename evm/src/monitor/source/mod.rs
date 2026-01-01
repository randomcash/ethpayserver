//! Block source abstraction for different providers.
//!
//! The `BlockSource` trait provides a unified interface for receiving
//! blockchain data from various sources (direct RPC, Alchemy, Infura, etc.)

pub mod rpc;
// pub mod alchemy;
// pub mod infura;

use crate::error::EvmResult;
use alloy::primitives::{Address, BlockNumber, U256, B256};
use alloy::rpc::types::{Log, Block};
use async_trait::async_trait;
use std::pin::Pin;
use tokio_stream::Stream;

/// A native ETH transfer found in a block.
#[derive(Debug, Clone)]
pub struct NativeTransfer {
    /// Transaction hash.
    pub tx_hash: B256,
    /// Sender address.
    pub from: Address,
    /// Recipient address.
    pub to: Address,
    /// Amount transferred in wei.
    pub value: U256,
    /// Transaction index in block.
    pub tx_index: u64,
}

/// Notification of a new block.
#[derive(Debug, Clone)]
pub struct BlockNotification {
    /// Block number.
    pub number: u64,
    /// Block hash.
    pub hash: B256,
    /// Parent block hash (for reorg detection).
    pub parent_hash: B256,
    /// Block timestamp.
    pub timestamp: u64,
}

impl From<&Block> for BlockNotification {
    fn from(block: &Block) -> Self {
        Self {
            number: block.header.number,
            hash: block.header.hash,
            parent_hash: block.header.parent_hash,
            timestamp: block.header.timestamp,
        }
    }
}

/// Filter for querying logs.
#[derive(Debug, Clone, Default)]
pub struct LogFilter {
    /// Contract addresses to filter (empty = all).
    pub addresses: Vec<Address>,
    /// Topic filters (ERC20 Transfer = topic0).
    pub topics: Vec<Option<B256>>,
    /// From block (inclusive).
    pub from_block: Option<BlockNumber>,
    /// To block (inclusive).
    pub to_block: Option<BlockNumber>,
}

impl LogFilter {
    /// Create a filter for ERC20 Transfer events to specific addresses.
    pub fn erc20_transfers_to(_addresses: Vec<Address>) -> Self {
        // Transfer(address indexed from, address indexed to, uint256 value)
        // topic0 = keccak256("Transfer(address,address,uint256)")
        let transfer_topic: B256 = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
            .parse()
            .expect("valid transfer topic");

        Self {
            addresses: vec![], // All token contracts
            topics: vec![
                Some(transfer_topic),  // topic0: Transfer event
                None,                   // topic1: from (any)
                // topic2 would be 'to', but we filter after
            ],
            from_block: None,
            to_block: None,
        }
    }

    /// Create a filter for a specific block range.
    pub fn with_block_range(mut self, from: u64, to: u64) -> Self {
        self.from_block = Some(from);
        self.to_block = Some(to);
        self
    }

    /// Add contract address filter.
    pub fn with_addresses(mut self, addresses: Vec<Address>) -> Self {
        self.addresses = addresses;
        self
    }
}

/// Source status for health checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceStatus {
    /// Connected and receiving blocks.
    Connected,
    /// Attempting to connect/reconnect.
    Connecting,
    /// Disconnected, will retry.
    Disconnected,
    /// Fatal error, stopped.
    Failed(String),
}

/// Block stream type alias.
pub type BlockStream = Pin<Box<dyn Stream<Item = EvmResult<BlockNotification>> + Send>>;

/// Trait for blockchain data sources.
///
/// Implementations provide block notifications and data fetching
/// from various sources (direct RPC, Alchemy, Infura, etc.)
#[async_trait]
pub trait BlockSource: Send + Sync {
    /// Get the chain ID this source is connected to.
    fn chain_id(&self) -> u64;

    /// Get the current connection status.
    fn status(&self) -> SourceStatus;

    /// Subscribe to new block notifications.
    ///
    /// Returns a stream of block notifications. The stream should
    /// automatically reconnect on connection failures.
    async fn subscribe_blocks(&self) -> EvmResult<BlockStream>;

    /// Get logs matching the filter.
    ///
    /// Used to query Transfer events for payment detection.
    async fn get_logs(&self, filter: &LogFilter) -> EvmResult<Vec<Log>>;

    /// Get the native balance of an address at a specific block.
    async fn get_balance(&self, address: Address, block: Option<u64>) -> EvmResult<U256>;

    /// Get the current block number.
    async fn get_block_number(&self) -> EvmResult<u64>;

    /// Get a block by number.
    async fn get_block(&self, number: u64) -> EvmResult<Option<Block>>;

    /// Find native ETH transfers to specific addresses in a block.
    ///
    /// Uses batch RPC to efficiently find transactions that sent ETH
    /// to any of the watched addresses. Returns matching transactions
    /// with full details (tx_hash, from, value).
    ///
    /// This is called only when a balance change is detected, making it
    /// efficient for payment detection without downloading every block.
    async fn find_native_transfers_to(
        &self,
        block_number: u64,
        addresses: &[Address],
    ) -> EvmResult<Vec<NativeTransfer>>;

    /// Get the number of confirmations for a block.
    ///
    /// Returns `current_block - block_number + 1`, or 0 if block not found.
    async fn get_confirmations(&self, block_number: u64) -> EvmResult<u64> {
        let current = self.get_block_number().await?;
        if block_number > current {
            Ok(0)
        } else {
            Ok(current - block_number + 1)
        }
    }

    /// Check if the source is healthy and connected.
    async fn is_healthy(&self) -> bool {
        self.status() == SourceStatus::Connected && self.get_block_number().await.is_ok()
    }
}
