//! Direct RPC connection block source.
//!
//! This implementation connects directly to an Ethereum node via
//! WebSocket for block subscriptions and HTTP for data fetching.

use super::{BlockNotification, BlockSource, BlockStream, LogFilter, NativeTransfer, SourceStatus};
use crate::error::{EvmError, EvmResult};
use crate::metrics as rpc_metrics;
use alloy::consensus::Transaction as TransactionTrait;
use alloy::network::TransactionResponse;
use alloy::primitives::{Address, U256};
use alloy::providers::{Provider, ProviderBuilder, RootProvider};
use alloy::rpc::types::{Block, BlockNumberOrTag, BlockTransactionsKind, Filter, Log};
use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use std::collections::HashSet;
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

/// Derive the printable origin (`scheme://host[:port]`) of an RPC endpoint.
///
/// The endpoints are [`SecretString`], so this is the only way to get anything
/// loggable out of one: the type makes leaking a full URL an explicit
/// `expose_secret()` rather than something a future `info!` does by accident.
/// Provider URLs carry the API key in the path (`.../v2/<key>`) or the query
/// (`?api-key=<key>`), so everything past the origin is secret.
fn redact_url(raw: &str) -> String {
    match Url::parse(raw) {
        Ok(url) => {
            let host = url.host_str().unwrap_or("<unknown-host>");
            match url.port() {
                Some(port) => format!("{}://{}:{}", url.scheme(), host, port),
                None => format!("{}://{}", url.scheme(), host),
            }
        }
        Err(_) => "<unparseable-url>".to_string(),
    }
}

/// Render a provider error with any occurrence of `url` replaced by its redacted
/// form. Alloy embeds the endpoint it was given in connection errors, so an
/// unfiltered error string leaks the key just as a log line would.
fn redact_err(e: impl std::fmt::Display, url: &str) -> String {
    e.to_string().replace(url, &redact_url(url))
}

/// Configuration for RPC block source.
#[derive(Debug, Clone)]
pub struct RpcSourceConfig {
    /// WebSocket URL for subscriptions. Secret: carries the provider API key.
    pub ws_url: Option<SecretString>,
    /// HTTP URL for RPC calls (required). Secret: carries the provider API key.
    pub http_url: SecretString,
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
    pub fn with_websocket(
        ws_url: impl Into<String>,
        http_url: impl Into<String>,
        chain_id: u64,
    ) -> Self {
        Self {
            ws_url: Some(SecretString::from(ws_url.into())),
            http_url: SecretString::from(http_url.into()),
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
            http_url: SecretString::from(http_url.into()),
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
        let _http_url = Url::parse(config.http_url.expose_secret())
            .map_err(|e| EvmError::InvalidChainConfig(format!("invalid HTTP URL: {}", e)))?;

        // Validate WS URL if provided
        if let Some(ref ws_url) = config.ws_url {
            let _ws_url = Url::parse(ws_url.expose_secret()).map_err(|e| {
                EvmError::InvalidChainConfig(format!("invalid WebSocket URL: {}", e))
            })?;
        }

        // Create HTTP provider
        let http_provider = ProviderBuilder::new()
            .disable_recommended_fillers()
            .connect(config.http_url.expose_secret())
            .await
            .map_err(|e| {
                EvmError::Connection(format!(
                    "HTTP connection failed: {}",
                    redact_err(e, config.http_url.expose_secret())
                ))
            })?;

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

        info!(
            chain_id,
            http_url = %redact_url(config.http_url.expose_secret()),
            "RPC block source created"
        );

        Ok(Self {
            config,
            http_provider,
            status: AtomicU8::new(STATUS_DISCONNECTED),
            error_message: RwLock::new(None),
        })
    }

    /// Create a WebSocket block stream.
    async fn create_ws_stream(&self) -> EvmResult<BlockStream> {
        let ws_url = self
            .config
            .ws_url
            .as_ref()
            .ok_or_else(|| EvmError::Connection("WebSocket URL not configured".to_string()))?;

        self.status.store(STATUS_CONNECTING, Ordering::SeqCst);

        // Connect via WebSocket
        let ws_provider = ProviderBuilder::new()
            .disable_recommended_fillers()
            .connect(ws_url.expose_secret())
            .await
            .map_err(|e| {
                self.status.store(STATUS_DISCONNECTED, Ordering::SeqCst);
                EvmError::Connection(format!(
                    "WebSocket connection failed: {}",
                    redact_err(e, ws_url.expose_secret())
                ))
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
        info!(
            chain_id,
            ws_url = %redact_url(ws_url.expose_secret()),
            "WebSocket connected"
        );

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
        let http_url = self.config.http_url.expose_secret().to_string();
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
                let msg = self
                    .error_message
                    .try_read()
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
            info!(
                chain_id = self.config.chain_id,
                "using HTTP polling for blocks"
            );
            Ok(self.create_polling_stream())
        } else {
            Err(EvmError::Connection(
                "no block source available".to_string(),
            ))
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

        let chain_id = self.config.chain_id;
        rpc_metrics::timed_rpc(chain_id, "get_logs", async {
            self.http_provider
                .get_logs(&alloy_filter)
                .await
                .map_err(|e| EvmError::Rpc(format!("get_logs failed: {}", e)))
        })
        .await
    }

    async fn get_balance(&self, address: Address, block: Option<u64>) -> EvmResult<U256> {
        let block_id = block
            .map(BlockNumberOrTag::Number)
            .unwrap_or(BlockNumberOrTag::Latest);

        let chain_id = self.config.chain_id;
        rpc_metrics::timed_rpc(chain_id, "get_balance", async {
            self.http_provider
                .get_balance(address)
                .block_id(block_id.into())
                .await
                .map_err(|e| EvmError::Rpc(format!("get_balance failed: {}", e)))
        })
        .await
    }

    async fn get_block_number(&self) -> EvmResult<u64> {
        let chain_id = self.config.chain_id;
        rpc_metrics::timed_rpc(chain_id, "get_block_number", async {
            self.http_provider
                .get_block_number()
                .await
                .map_err(|e| EvmError::Rpc(format!("get_block_number failed: {}", e)))
        })
        .await
    }

    async fn get_block(&self, number: u64) -> EvmResult<Option<Block>> {
        let chain_id = self.config.chain_id;
        rpc_metrics::timed_rpc(chain_id, "get_block", async {
            self.http_provider
                .get_block_by_number(BlockNumberOrTag::Number(number))
                .await
                .map_err(|e| EvmError::Rpc(format!("get_block failed: {}", e)))
        })
        .await
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

        // Fetch block with full transactions (instrumented)
        let chain_id = self.config.chain_id;
        let block = rpc_metrics::timed_rpc(chain_id, "get_block_with_txs", async {
            self.http_provider
                .get_block_by_number(BlockNumberOrTag::Number(block_number))
                .kind(BlockTransactionsKind::Full)
                .await
                .map_err(|e| EvmError::Rpc(format!("get_block_with_txs failed: {}", e)))
        })
        .await?;

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
    fn config_debug_does_not_leak_the_api_key() {
        // The point of SecretString: this holds without anyone remembering to
        // call a redaction helper at the log site.
        let config = RpcSourceConfig::with_websocket(
            "wss://eth-sepolia.g.alchemy.com/v2/alch_supersecretkey",
            "https://eth-sepolia.g.alchemy.com/v2/alch_supersecretkey",
            11155111,
        );
        let rendered = format!("{:?}", config);
        assert!(
            !rendered.contains("alch_supersecretkey"),
            "Debug leaked the endpoint: {rendered}"
        );
    }

    #[test]
    fn redact_url_drops_api_key_path() {
        assert_eq!(
            redact_url("https://eth-sepolia.g.alchemy.com/v2/alch_supersecretkey"),
            "https://eth-sepolia.g.alchemy.com"
        );
        assert_eq!(
            redact_url("wss://eth-sepolia.g.alchemy.com/v2/alch_supersecretkey"),
            "wss://eth-sepolia.g.alchemy.com"
        );
    }

    #[test]
    fn redact_url_drops_api_key_query() {
        assert_eq!(
            redact_url("https://rpc.example.com/rpc?api-key=secret123"),
            "https://rpc.example.com"
        );
    }

    #[test]
    fn redact_url_keeps_port_for_self_hosted_nodes() {
        assert_eq!(
            redact_url("http://192.168.1.10:8545/"),
            "http://192.168.1.10:8545"
        );
    }

    #[test]
    fn redact_url_handles_garbage() {
        assert_eq!(redact_url("not a url"), "<unparseable-url>");
    }

    #[test]
    fn redact_err_scrubs_embedded_endpoint() {
        let url = "https://eth-sepolia.g.alchemy.com/v2/alch_supersecretkey";
        let rendered = redact_err(format!("error sending request for url ({})", url), url);
        assert!(!rendered.contains("alch_supersecretkey"));
        assert!(rendered.contains("https://eth-sepolia.g.alchemy.com"));
    }

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
