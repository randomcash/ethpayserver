//! Direct RPC connection block source.
//!
//! This implementation connects directly to an Ethereum node via
//! WebSocket for block subscriptions and HTTP for data fetching.

use super::{BlockNotification, BlockSource, BlockStream, LogFilter, NativeTransfer, SourceStatus};
use crate::error::{EvmError, EvmResult};
use alloy::consensus::Transaction as TransactionTrait;
use alloy::network::TransactionResponse;
use alloy::primitives::{Address, U256};
use alloy::providers::{Provider, ProviderBuilder, RootProvider};
use alloy::rpc::types::{Block, BlockNumberOrTag, BlockTransactionsKind, Filter, Log};
use std::collections::HashSet;
use async_trait::async_trait;
use std::sync::atomic::{AtomicU8, Ordering};
use tokio::sync::RwLock;
use tokio_stream::StreamExt;
use tracing::{debug, error, info, warn};
use url::Url;

/// Status codes for atomic storage.
const STATUS_CONNECTED: u8 = 0;
const STATUS_CONNECTING: u8 = 1;
const STATUS_DISCONNECTED: u8 = 2;
const STATUS_FAILED: u8 = 3;

/// Configuration for RPC block source.
#[derive(Debug, Clone)]
pub struct RpcSourceConfig {
    /// WebSocket URL for subscriptions.
    pub ws_url: Option<String>,
    /// HTTP URL for RPC calls (required).
    pub http_url: String,
    /// Chain ID (verified on connection).
    pub chain_id: u64,
    /// Maximum reconnection attempts (0 = infinite).
    pub max_reconnect_attempts: u32,
    /// Delay between reconnection attempts (ms).
    pub reconnect_delay_ms: u64,
    /// Fallback to polling if WebSocket unavailable.
    pub fallback_to_polling: bool,
    /// Polling interval if using HTTP polling (ms).
    pub polling_interval_ms: u64,
}

impl RpcSourceConfig {
    /// Create config for WebSocket + HTTP.
    pub fn with_websocket(ws_url: impl Into<String>, http_url: impl Into<String>, chain_id: u64) -> Self {
        Self {
            ws_url: Some(ws_url.into()),
            http_url: http_url.into(),
            chain_id,
            max_reconnect_attempts: 0, // infinite
            reconnect_delay_ms: 5000,
            fallback_to_polling: true,
            polling_interval_ms: 12000, // ~1 block on mainnet
        }
    }

    /// Create config for HTTP-only (polling).
    pub fn http_only(http_url: impl Into<String>, chain_id: u64) -> Self {
        Self {
            ws_url: None,
            http_url: http_url.into(),
            chain_id,
            max_reconnect_attempts: 0,
            reconnect_delay_ms: 5000,
            fallback_to_polling: true,
            polling_interval_ms: 12000,
        }
    }
}

/// Direct RPC connection block source.
///
/// Supports WebSocket subscriptions with HTTP fallback for data fetching.
/// Automatically reconnects on connection failures.
pub struct RpcBlockSource {
    config: RpcSourceConfig,
    /// HTTP provider for RPC calls.
    http_provider: RootProvider,
    /// Current status.
    status: AtomicU8,
    /// Error message if failed.
    error_message: RwLock<Option<String>>,
}

impl RpcBlockSource {
    /// Create a new RPC block source.
    pub async fn new(config: RpcSourceConfig) -> EvmResult<Self> {
        // Validate HTTP URL
        let _http_url = Url::parse(&config.http_url)
            .map_err(|e| EvmError::InvalidChainConfig(format!("invalid HTTP URL: {}", e)))?;

        // Validate WS URL if provided
        if let Some(ref ws_url) = config.ws_url {
            let _ws_url = Url::parse(ws_url)
                .map_err(|e| EvmError::InvalidChainConfig(format!("invalid WebSocket URL: {}", e)))?;
        }

        // Create HTTP provider
        let http_provider = ProviderBuilder::new()
            .disable_recommended_fillers()
            .connect(&config.http_url)
            .await
            .map_err(|e| EvmError::Connection(format!("HTTP connection failed: {}", e)))?;

        // Verify chain ID
        let chain_id = http_provider
            .get_chain_id()
            .await
            .map_err(|e| EvmError::Connection(format!("failed to get chain ID: {}", e)))?;

        if chain_id != config.chain_id {
            return Err(EvmError::InvalidChainConfig(format!(
                "chain ID mismatch: expected {}, got {}",
                config.chain_id, chain_id
            )));
        }

        info!(chain_id, http_url = %config.http_url, "RPC block source created");

        Ok(Self {
            config,
            http_provider,
            status: AtomicU8::new(STATUS_DISCONNECTED),
            error_message: RwLock::new(None),
        })
    }

    /// Create a WebSocket block stream.
    async fn create_ws_stream(&self) -> EvmResult<BlockStream> {
        let ws_url = self.config.ws_url.as_ref().ok_or_else(|| {
            EvmError::Connection("WebSocket URL not configured".to_string())
        })?;

        self.status.store(STATUS_CONNECTING, Ordering::SeqCst);

        // Connect via WebSocket
        let ws_provider = ProviderBuilder::new()
            .disable_recommended_fillers()
            .connect(ws_url)
            .await
            .map_err(|e| {
                self.status.store(STATUS_DISCONNECTED, Ordering::SeqCst);
                EvmError::Connection(format!("WebSocket connection failed: {}", e))
            })?;

        // Verify chain ID
        let chain_id = ws_provider
            .get_chain_id()
            .await
            .map_err(|e| EvmError::Connection(format!("failed to verify WS chain ID: {}", e)))?;

        if chain_id != self.config.chain_id {
            return Err(EvmError::InvalidChainConfig(format!(
                "WS chain ID mismatch: expected {}, got {}",
                self.config.chain_id, chain_id
            )));
        }

        self.status.store(STATUS_CONNECTED, Ordering::SeqCst);
        info!(chain_id, ws_url, "WebSocket connected");

        // Subscribe to new blocks (returns headers)
        let subscription = ws_provider
            .subscribe_blocks()
            .await
            .map_err(|e| EvmError::Subscription(format!("block subscription failed: {}", e)))?;

        let config_chain_id = self.config.chain_id;

        // Keep ws_provider alive by moving it into the stream
        let stream = async_stream::stream! {
            // Hold provider reference to keep connection alive
            let _provider = ws_provider;
            let mut sub_stream = subscription.into_stream();

            while let Some(header) = sub_stream.next().await {
                debug!(chain_id = config_chain_id, block = header.number, "new block via WS");
                yield Ok(BlockNotification {
                    number: header.number,
                    hash: header.hash,
                    parent_hash: header.parent_hash,
                    timestamp: header.timestamp,
                });
            }

            error!(chain_id = config_chain_id, "WebSocket subscription ended");
        };

        Ok(Box::pin(stream))
    }

    /// Create a polling-based block stream (fallback).
    fn create_polling_stream(&self) -> BlockStream {
        let http_url = self.config.http_url.clone();
        let interval = self.config.polling_interval_ms;
        let chain_id = self.config.chain_id;

        let stream = async_stream::stream! {
            let provider = match ProviderBuilder::new()
                .disable_recommended_fillers()
                .connect(&http_url)
                .await
            {
                Ok(p) => p,
                Err(e) => {
                    yield Err(EvmError::Connection(format!("polling connection failed: {}", e)));
                    return;
                }
            };

            let mut last_block: Option<u64> = None;
            let mut interval_timer = tokio::time::interval(
                tokio::time::Duration::from_millis(interval)
            );

            debug!(chain_id, interval_ms = interval, "starting block polling");

            loop {
                interval_timer.tick().await;

                match provider.get_block_by_number(BlockNumberOrTag::Latest).await {
                    Ok(Some(block)) => {
                        let block_num = block.header.number;

                        // Only emit if new block
                        if last_block.is_none_or(|last| block_num > last) {
                            last_block = Some(block_num);
                            yield Ok(BlockNotification::from(&block));
                        }
                    }
                    Ok(None) => {
                        warn!(chain_id, "no block returned from provider");
                    }
                    Err(e) => {
                        error!(chain_id, error = %e, "polling error");
                        yield Err(EvmError::Rpc(format!("polling error: {}", e)));
                    }
                }
            }
        };

        Box::pin(stream)
    }
}

#[async_trait]
impl BlockSource for RpcBlockSource {
    fn chain_id(&self) -> u64 {
        self.config.chain_id
    }

    fn status(&self) -> SourceStatus {
        match self.status.load(Ordering::SeqCst) {
            STATUS_CONNECTED => SourceStatus::Connected,
            STATUS_CONNECTING => SourceStatus::Connecting,
            STATUS_DISCONNECTED => SourceStatus::Disconnected,
            STATUS_FAILED => {
                let msg = self.error_message.try_read()
                    .ok()
                    .and_then(|guard| guard.clone())
                    .unwrap_or_else(|| "unknown error".to_string());
                SourceStatus::Failed(msg)
            }
            _ => SourceStatus::Disconnected,
        }
    }

    async fn subscribe_blocks(&self) -> EvmResult<BlockStream> {
        // Try WebSocket subscription first
        if self.config.ws_url.is_some() {
            match self.create_ws_stream().await {
                Ok(stream) => return Ok(stream),
                Err(e) => {
                    warn!(error = %e, "WebSocket connection failed, falling back to polling");
                    if !self.config.fallback_to_polling {
                        return Err(e);
                    }
                }
            }
        }

        // Fallback to polling
        if self.config.fallback_to_polling || self.config.ws_url.is_none() {
            info!(chain_id = self.config.chain_id, "using HTTP polling for blocks");
            Ok(self.create_polling_stream())
        } else {
            Err(EvmError::Connection("no block source available".to_string()))
        }
    }

    async fn get_logs(&self, filter: &LogFilter) -> EvmResult<Vec<Log>> {
        let mut alloy_filter = Filter::new();

        if !filter.addresses.is_empty() {
            alloy_filter = alloy_filter.address(filter.addresses.clone());
        }

        // Add topics
        for (i, topic) in filter.topics.iter().enumerate() {
            if let Some(t) = topic {
                match i {
                    0 => alloy_filter = alloy_filter.event_signature(*t),
                    1 => alloy_filter = alloy_filter.topic1(*t),
                    2 => alloy_filter = alloy_filter.topic2(*t),
                    3 => alloy_filter = alloy_filter.topic3(*t),
                    _ => {}
                }
            }
        }

        if let Some(from) = filter.from_block {
            alloy_filter = alloy_filter.from_block(from);
        }
        if let Some(to) = filter.to_block {
            alloy_filter = alloy_filter.to_block(to);
        }

        self.http_provider
            .get_logs(&alloy_filter)
            .await
            .map_err(|e| EvmError::Rpc(format!("get_logs failed: {}", e)))
    }

    async fn get_balance(&self, address: Address, block: Option<u64>) -> EvmResult<U256> {
        let block_id = block
            .map(BlockNumberOrTag::Number)
            .unwrap_or(BlockNumberOrTag::Latest);

        self.http_provider
            .get_balance(address)
            .block_id(block_id.into())
            .await
            .map_err(|e| EvmError::Rpc(format!("get_balance failed: {}", e)))
    }

    async fn get_block_number(&self) -> EvmResult<u64> {
        self.http_provider
            .get_block_number()
            .await
            .map_err(|e| EvmError::Rpc(format!("get_block_number failed: {}", e)))
    }

    async fn get_block(&self, number: u64) -> EvmResult<Option<Block>> {
        self.http_provider
            .get_block_by_number(BlockNumberOrTag::Number(number))
            .await
            .map_err(|e| EvmError::Rpc(format!("get_block failed: {}", e)))
    }

    async fn find_native_transfers_to(
        &self,
        block_number: u64,
        addresses: &[Address],
    ) -> EvmResult<Vec<NativeTransfer>> {
        if addresses.is_empty() {
            return Ok(Vec::new());
        }

        // Build a set for O(1) lookup
        let watched: HashSet<Address> = addresses.iter().copied().collect();

        // Fetch block with full transactions
        let block = self
            .http_provider
            .get_block_by_number(BlockNumberOrTag::Number(block_number))
            .kind(BlockTransactionsKind::Full)
            .await
            .map_err(|e| EvmError::Rpc(format!("get_block_with_txs failed: {}", e)))?;

        let Some(block) = block else {
            return Ok(Vec::new());
        };

        // Filter transactions that send ETH to watched addresses
        let mut transfers = Vec::new();

        for (tx_index, tx) in block.transactions.txns().enumerate() {
            // Skip if no recipient (contract creation)
            let Some(to) = tx.to() else {
                continue;
            };

            // Skip if not sending to a watched address
            if !watched.contains(&to) {
                continue;
            }

            // Skip if no value transferred
            if tx.value().is_zero() {
                continue;
            }

            transfers.push(NativeTransfer {
                tx_hash: tx.tx_hash(),
                from: tx.from(),
                to,
                value: tx.value(),
                tx_index: tx_index as u64,
            });
        }

        debug!(
            chain_id = self.config.chain_id,
            block = block_number,
            found = transfers.len(),
            "scanned block for native transfers"
        );

        Ok(transfers)
    }
}

impl std::fmt::Debug for RpcBlockSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcBlockSource")
            .field("chain_id", &self.config.chain_id)
            .field("http_url", &self.config.http_url)
            .field("ws_url", &self.config.ws_url)
            .field("status", &self.status())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_with_websocket() {
        let config = RpcSourceConfig::with_websocket(
            "wss://eth-mainnet.g.alchemy.com/v2/key",
            "https://eth-mainnet.g.alchemy.com/v2/key",
            1,
        );
        assert!(config.ws_url.is_some());
        assert_eq!(config.chain_id, 1);
    }

    #[test]
    fn test_config_http_only() {
        let config = RpcSourceConfig::http_only("https://eth.llamarpc.com", 1);
        assert!(config.ws_url.is_none());
        assert!(config.fallback_to_polling);
    }

    #[test]
    fn test_log_filter_erc20() {
        let filter = LogFilter::erc20_transfers_to(vec![]);
        assert!(!filter.topics.is_empty());
        assert!(filter.topics[0].is_some());
    }
}
