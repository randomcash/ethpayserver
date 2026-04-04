//! EVM Monitor service for communicating with evmmonitor.
//!
//! Sends WatchAddress/UnwatchAddress commands via Redis pub/sub.

use std::sync::Arc;

use async_trait::async_trait;
use evm::monitor::events::{MonitorCommand, UnwatchAddressCommand, WatchAddressCommand};
use evm::monitor::{COMMANDS_CHANNEL, ChainHealth, EVENTS_CHANNEL, EventBridge, RedisBridge};
use evm::{Address, network_to_chain_id};
use types::Network;
use uuid::Uuid;

/// Error type for EVM monitor operations.
#[derive(Debug, thiserror::Error)]
pub enum EVMMonitorError {
    #[error("unsupported network: {0:?}")]
    UnsupportedNetwork(Network),

    #[error("bridge error: {0}")]
    Bridge(#[from] evm::EvmError),
}

/// Interface for EVM payment monitoring.
///
/// Implementations send commands to the evmmonitor service to watch/unwatch
/// addresses for incoming payments.
#[async_trait]
pub trait EVMMonitor: Send + Sync {
    /// Start watching an address for incoming payments.
    async fn watch_address(
        &self,
        network: Network,
        address: Address,
        invoice_id: Uuid,
        expected_amount: Option<evm::U256>,
        token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError>;

    /// Start watching an address using chain_id directly (for testnets/custom chains).
    async fn watch_address_by_chain_id(
        &self,
        chain_id: u64,
        address: Address,
        invoice_id: Uuid,
        expected_amount: Option<evm::U256>,
        token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError>;

    /// Stop watching an address.
    async fn unwatch_address(
        &self,
        network: Network,
        address: Address,
        token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError>;

    /// Stop watching an address using chain_id directly.
    async fn unwatch_address_by_chain_id(
        &self,
        chain_id: u64,
        address: Address,
        token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError>;

    /// Check if the monitor connection is healthy.
    async fn health_check(&self) -> Result<(), EVMMonitorError>;

    /// Get chain health information from evmmonitor.
    ///
    /// Returns health info for all monitored chains.
    async fn get_chain_health(&self) -> Result<Vec<ChainHealth>, EVMMonitorError>;
}

/// Redis-based implementation of EVMMonitor.
///
/// Communicates with evmmonitor via Redis pub/sub channels.
pub struct RedisEVMMonitor {
    bridge: Arc<RedisBridge>,
}

impl Clone for RedisEVMMonitor {
    fn clone(&self) -> Self {
        Self {
            bridge: Arc::clone(&self.bridge),
        }
    }
}

impl RedisEVMMonitor {
    /// Create a new Redis-based EVM monitor.
    pub fn new(bridge: Arc<RedisBridge>) -> Self {
        Self { bridge }
    }

    /// Connect to Redis and create a new monitor.
    pub async fn connect(redis_url: &str) -> Result<Self, EVMMonitorError> {
        let bridge = RedisBridge::new(redis_url, EVENTS_CHANNEL, COMMANDS_CHANNEL).await?;
        Ok(Self::new(Arc::new(bridge)))
    }
}

#[async_trait]
impl EVMMonitor for RedisEVMMonitor {
    async fn watch_address(
        &self,
        network: Network,
        address: Address,
        invoice_id: Uuid,
        expected_amount: Option<evm::U256>,
        token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        let chain_id =
            network_to_chain_id(network).ok_or(EVMMonitorError::UnsupportedNetwork(network))?;

        self.watch_address_by_chain_id(
            chain_id,
            address,
            invoice_id,
            expected_amount,
            token_contract,
        )
        .await
    }

    async fn watch_address_by_chain_id(
        &self,
        chain_id: u64,
        address: Address,
        invoice_id: Uuid,
        expected_amount: Option<evm::U256>,
        token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        let command = MonitorCommand::WatchAddress(WatchAddressCommand {
            chain_id,
            address,
            invoice_id,
            expected_amount,
            token_contract,
        });

        self.bridge.publish_command(&command).await?;
        tracing::info!(
            chain_id,
            address = %address,
            invoice_id = %invoice_id,
            "sent WatchAddress command"
        );

        Ok(())
    }

    async fn unwatch_address(
        &self,
        network: Network,
        address: Address,
        token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        let chain_id =
            network_to_chain_id(network).ok_or(EVMMonitorError::UnsupportedNetwork(network))?;

        self.unwatch_address_by_chain_id(chain_id, address, token_contract)
            .await
    }

    async fn unwatch_address_by_chain_id(
        &self,
        chain_id: u64,
        address: Address,
        token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        let command = MonitorCommand::UnwatchAddress(UnwatchAddressCommand {
            chain_id,
            address,
            token_contract,
        });

        self.bridge.publish_command(&command).await?;
        tracing::info!(
            chain_id,
            address = %address,
            token_contract = ?token_contract,
            "sent UnwatchAddress command"
        );

        Ok(())
    }

    async fn health_check(&self) -> Result<(), EVMMonitorError> {
        self.bridge.health_check().await?;
        Ok(())
    }

    async fn get_chain_health(&self) -> Result<Vec<ChainHealth>, EVMMonitorError> {
        const HEALTH_KEY: &str = "evmmonitor:health";

        let health_json: Option<String> = self.bridge.get_key(HEALTH_KEY).await?;

        match health_json {
            Some(json) => serde_json::from_str(&json).map_err(|e| {
                EVMMonitorError::Bridge(evm::EvmError::Monitor(format!(
                    "failed to parse health JSON: {}",
                    e
                )))
            }),
            None => Ok(vec![]), // No health data yet
        }
    }
}
