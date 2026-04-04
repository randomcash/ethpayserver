//! RPC provider management for EVM chains.
//!
//! This module provides a wrapper around alloy's provider to interact
//! with EVM-compatible blockchains via JSON-RPC.

use crate::error::{EvmError, EvmResult};
use crate::network::ChainConfig;
use alloy::primitives::{Address, U256};
use alloy::providers::{Provider, ProviderBuilder, RootProvider};
use std::sync::Arc;
use url::Url;

/// RPC provider for interacting with an EVM chain.
#[derive(Clone)]
pub struct EvmProvider {
    /// The underlying alloy provider.
    provider: Arc<RootProvider>,
    /// Chain configuration.
    chain_config: &'static ChainConfig,
    /// RPC endpoint URL.
    rpc_url: Url,
}

impl EvmProvider {
    /// Create a new provider for the given chain and RPC URL.
    pub async fn new(chain_config: &'static ChainConfig, rpc_url: &str) -> EvmResult<Self> {
        let url = Url::parse(rpc_url).map_err(|e| EvmError::InvalidChainConfig(e.to_string()))?;

        // Create a basic RootProvider without fillers
        let provider = ProviderBuilder::new()
            .disable_recommended_fillers()
            .connect(rpc_url)
            .await
            .map_err(|e| EvmError::Connection(e.to_string()))?;

        // Verify chain ID
        let chain_id = provider
            .get_chain_id()
            .await
            .map_err(|e| EvmError::Connection(e.to_string()))?;

        if chain_id != chain_config.chain_id {
            return Err(EvmError::InvalidChainConfig(format!(
                "chain ID mismatch: expected {}, got {}",
                chain_config.chain_id, chain_id
            )));
        }

        Ok(Self {
            provider: Arc::new(provider),
            chain_config,
            rpc_url: url,
        })
    }

    /// Get the chain configuration.
    pub fn chain_config(&self) -> &'static ChainConfig {
        self.chain_config
    }

    /// Get the RPC URL.
    pub fn rpc_url(&self) -> &Url {
        &self.rpc_url
    }

    /// Get the current block number.
    pub async fn get_block_number(&self) -> EvmResult<u64> {
        self.provider
            .get_block_number()
            .await
            .map_err(|e| EvmError::Rpc(e.to_string()))
    }

    /// Get the native token balance (ETH, MATIC, etc.) for an address.
    pub async fn get_native_balance(&self, address: Address) -> EvmResult<U256> {
        self.provider
            .get_balance(address)
            .await
            .map_err(|e| EvmError::Rpc(e.to_string()))
    }

    /// Get the transaction count (nonce) for an address.
    pub async fn get_transaction_count(&self, address: Address) -> EvmResult<u64> {
        self.provider
            .get_transaction_count(address)
            .await
            .map_err(|e| EvmError::Rpc(e.to_string()))
    }

    /// Check if the provider is connected and responding.
    pub async fn is_healthy(&self) -> bool {
        self.provider.get_block_number().await.is_ok()
    }

    /// Get access to the underlying alloy provider for advanced operations.
    pub fn inner(&self) -> &RootProvider {
        &self.provider
    }
}

impl std::fmt::Debug for EvmProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvmProvider")
            .field("chain", &self.chain_config.name)
            .field("chain_id", &self.chain_config.chain_id)
            .field("rpc_url", &self.rpc_url.as_str())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::ETHEREUM;

    // These tests require a live RPC endpoint, so they're ignored by default.
    // Run with: cargo test -- --ignored

    #[tokio::test]
    #[ignore = "requires live RPC endpoint"]
    async fn test_provider_connection() {
        let provider = EvmProvider::new(&ETHEREUM, "https://eth.llamarpc.com").await;
        assert!(provider.is_ok());
    }

    #[tokio::test]
    #[ignore = "requires live RPC endpoint"]
    async fn test_get_block_number() {
        let provider = EvmProvider::new(&ETHEREUM, "https://eth.llamarpc.com")
            .await
            .unwrap();
        let block = provider.get_block_number().await.unwrap();
        assert!(block > 0);
    }

    #[tokio::test]
    #[ignore = "requires live RPC endpoint"]
    async fn test_get_balance() {
        let provider = EvmProvider::new(&ETHEREUM, "https://eth.llamarpc.com")
            .await
            .unwrap();

        // Vitalik's address
        let address: Address = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"
            .parse()
            .unwrap();
        let balance = provider.get_native_balance(address).await.unwrap();

        // Should have some ETH
        assert!(balance > U256::ZERO);
    }
}
