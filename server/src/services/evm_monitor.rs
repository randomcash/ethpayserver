//! EVM Monitor service for communicating with evmmonitor.
//!
//! Sends WatchAddress/UnwatchAddress commands via Redis pub/sub.

use std::sync::Arc;

use async_trait::async_trait;
use evm::Address;
use evm::monitor::events::{MonitorCommand, UnwatchAddressCommand, WatchAddressCommand};
use evm::monitor::{COMMANDS_CHANNEL, ChainHealth, EVENTS_CHANNEL, EventBridge, RedisBridge};
use types::ChainId;
use uuid::Uuid;

/// Error type for EVM monitor operations.
#[derive(Debug, thiserror::Error)]
pub enum EVMMonitorError {
    #[error("not an EVM chain: {0}")]
    NotAnEvmChain(ChainId),

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
        chain_id: &ChainId,
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
        chain_id: &ChainId,
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

    /// Get the `SENTRY_RELEASE` evmmonitor was compiled with, observed live
    /// from the running process rather than trusted from the build log.
    ///
    /// Defaults to `None`: evmmonitor is the only implementation that can
    /// observe this at all, so every other implementation (test doubles)
    /// stays unaffected by this method existing.
    async fn get_sentry_release(&self) -> Result<Option<String>, EVMMonitorError> {
        Ok(None)
    }
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
        chain_id: &ChainId,
        address: Address,
        invoice_id: Uuid,
        expected_amount: Option<evm::U256>,
        token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        // This monitor talks to EVM RPCs, which take an EIP-155 number. That
        // is the only place the number is still the right representation.
        let eip155 = chain_id
            .evm_chain_id()
            .ok_or_else(|| EVMMonitorError::NotAnEvmChain(chain_id.clone()))?;

        self.watch_address_by_chain_id(eip155, address, invoice_id, expected_amount, token_contract)
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
        chain_id: &ChainId,
        address: Address,
        token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        // This monitor talks to EVM RPCs, which take an EIP-155 number. That
        // is the only place the number is still the right representation.
        let eip155 = chain_id
            .evm_chain_id()
            .ok_or_else(|| EVMMonitorError::NotAnEvmChain(chain_id.clone()))?;

        self.unwatch_address_by_chain_id(eip155, address, token_contract)
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

    async fn get_sentry_release(&self) -> Result<Option<String>, EVMMonitorError> {
        const SENTRY_RELEASE_KEY: &str = "evmmonitor:sentry_release";

        let release: Option<String> = self.bridge.get_key(SENTRY_RELEASE_KEY).await?;
        Ok(observed_sentry_release(release))
    }
}

/// An unset `SENTRY_RELEASE` is published as the empty string (see
/// evmmonitor's health publisher), not omitted - treat it the same as "not
/// observed" rather than as a release worth comparing against. Split out from
/// [`RedisEVMMonitor::get_sentry_release`] so this is unit-testable without a
/// live Redis.
fn observed_sentry_release(published: Option<String>) -> Option<String> {
    published.filter(|r| !r.is_empty())
}

#[cfg(test)]
mod tests {
    use super::observed_sentry_release;

    #[test]
    fn a_non_empty_published_release_is_observed() {
        assert_eq!(
            observed_sentry_release(Some("abc1234".to_string())),
            Some("abc1234".to_string())
        );
    }

    #[test]
    fn an_empty_published_release_is_treated_as_not_observed() {
        assert_eq!(observed_sentry_release(Some(String::new())), None);
    }

    #[test]
    fn no_published_value_is_not_observed() {
        assert_eq!(observed_sentry_release(None), None);
    }
}
