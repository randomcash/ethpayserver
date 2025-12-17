//! EVM chain configurations and traits.
//!
//! This module defines the supported EVM chains and their configurations.
//! Each chain has specific parameters like chain ID, RPC endpoints, block time, etc.

use serde::{Deserialize, Serialize};

/// Supported EVM-compatible networks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvmNetwork {
    /// Ethereum mainnet
    Ethereum,
    /// Polygon (formerly Matic)
    Polygon,
    /// Arbitrum One
    Arbitrum,
    /// Optimism
    Optimism,
    /// Base (Coinbase L2)
    Base,
    /// Avalanche C-Chain
    Avalanche,
    /// BNB Smart Chain (formerly BSC)
    BinanceSmartChain,
    /// zkSync Era
    ZkSync,
    /// Linea
    Linea,
    /// Scroll
    Scroll,
}

impl EvmNetwork {
    /// Returns the display name for this network.
    pub fn display_name(&self) -> &'static str {
        match self {
            EvmNetwork::Ethereum => "Ethereum",
            EvmNetwork::Polygon => "Polygon",
            EvmNetwork::Arbitrum => "Arbitrum",
            EvmNetwork::Optimism => "Optimism",
            EvmNetwork::Base => "Base",
            EvmNetwork::Avalanche => "Avalanche",
            EvmNetwork::BinanceSmartChain => "BNB Chain",
            EvmNetwork::ZkSync => "zkSync",
            EvmNetwork::Linea => "Linea",
            EvmNetwork::Scroll => "Scroll",
        }
    }

    /// Returns the native currency symbol for this network.
    pub fn native_symbol(&self) -> &'static str {
        match self {
            EvmNetwork::Ethereum => "ETH",
            EvmNetwork::Polygon => "POL",
            EvmNetwork::Arbitrum => "ETH",
            EvmNetwork::Optimism => "ETH",
            EvmNetwork::Base => "ETH",
            EvmNetwork::Avalanche => "AVAX",
            EvmNetwork::BinanceSmartChain => "BNB",
            EvmNetwork::ZkSync => "ETH",
            EvmNetwork::Linea => "ETH",
            EvmNetwork::Scroll => "ETH",
        }
    }

    /// Returns the EIP-155 chain ID for this network.
    pub fn chain_id(&self) -> u64 {
        match self {
            EvmNetwork::Ethereum => 1,
            EvmNetwork::Polygon => 137,
            EvmNetwork::Arbitrum => 42161,
            EvmNetwork::Optimism => 10,
            EvmNetwork::Base => 8453,
            EvmNetwork::Avalanche => 43114,
            EvmNetwork::BinanceSmartChain => 56,
            EvmNetwork::ZkSync => 324,
            EvmNetwork::Linea => 59144,
            EvmNetwork::Scroll => 534352,
        }
    }

    /// Get network from chain ID.
    pub fn from_chain_id(chain_id: u64) -> Option<Self> {
        match chain_id {
            1 => Some(EvmNetwork::Ethereum),
            137 => Some(EvmNetwork::Polygon),
            42161 => Some(EvmNetwork::Arbitrum),
            10 => Some(EvmNetwork::Optimism),
            8453 => Some(EvmNetwork::Base),
            43114 => Some(EvmNetwork::Avalanche),
            56 => Some(EvmNetwork::BinanceSmartChain),
            324 => Some(EvmNetwork::ZkSync),
            59144 => Some(EvmNetwork::Linea),
            534352 => Some(EvmNetwork::Scroll),
            _ => None,
        }
    }
}

impl std::fmt::Display for EvmNetwork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Configuration for an EVM-compatible chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainConfig {
    /// The network identifier.
    pub network: EvmNetwork,
    /// EIP-155 chain ID.
    pub chain_id: u64,
    /// Human-readable chain name.
    pub name: &'static str,
    /// Native currency symbol (e.g., "ETH", "MATIC").
    pub native_symbol: &'static str,
    /// Native currency decimals (always 18 for EVM chains).
    pub native_decimals: u8,
    /// Average block time in seconds.
    pub block_time_secs: u64,
    /// Number of confirmations required for finality.
    pub confirmations_required: u32,
    /// Block explorer URL template (use {tx} for tx hash, {address} for address).
    pub explorer_url: &'static str,
}

impl ChainConfig {
    /// Get the explorer URL for a transaction.
    pub fn tx_explorer_url(&self, tx_hash: &str) -> String {
        self.explorer_url.replace("{tx}", tx_hash)
    }

    /// Get the explorer URL for an address.
    pub fn address_explorer_url(&self, address: &str) -> String {
        self.explorer_url
            .replace("/tx/{tx}", &format!("/address/{}", address))
    }
}

/// Ethereum Mainnet configuration.
pub const ETHEREUM: ChainConfig = ChainConfig {
    network: EvmNetwork::Ethereum,
    chain_id: 1,
    name: "Ethereum",
    native_symbol: "ETH",
    native_decimals: 18,
    block_time_secs: 12,
    confirmations_required: 12,
    explorer_url: "https://etherscan.io/tx/{tx}",
};

/// Polygon (formerly Matic) configuration.
pub const POLYGON: ChainConfig = ChainConfig {
    network: EvmNetwork::Polygon,
    chain_id: 137,
    name: "Polygon",
    native_symbol: "POL",
    native_decimals: 18,
    block_time_secs: 2,
    confirmations_required: 128,
    explorer_url: "https://polygonscan.com/tx/{tx}",
};

/// Arbitrum One configuration.
pub const ARBITRUM: ChainConfig = ChainConfig {
    network: EvmNetwork::Arbitrum,
    chain_id: 42161,
    name: "Arbitrum One",
    native_symbol: "ETH",
    native_decimals: 18,
    block_time_secs: 1,
    confirmations_required: 64,
    explorer_url: "https://arbiscan.io/tx/{tx}",
};

/// Optimism configuration.
pub const OPTIMISM: ChainConfig = ChainConfig {
    network: EvmNetwork::Optimism,
    chain_id: 10,
    name: "Optimism",
    native_symbol: "ETH",
    native_decimals: 18,
    block_time_secs: 2,
    confirmations_required: 64,
    explorer_url: "https://optimistic.etherscan.io/tx/{tx}",
};

/// Base (Coinbase L2) configuration.
pub const BASE: ChainConfig = ChainConfig {
    network: EvmNetwork::Base,
    chain_id: 8453,
    name: "Base",
    native_symbol: "ETH",
    native_decimals: 18,
    block_time_secs: 2,
    confirmations_required: 64,
    explorer_url: "https://basescan.org/tx/{tx}",
};

/// Avalanche C-Chain configuration.
pub const AVALANCHE: ChainConfig = ChainConfig {
    network: EvmNetwork::Avalanche,
    chain_id: 43114,
    name: "Avalanche C-Chain",
    native_symbol: "AVAX",
    native_decimals: 18,
    block_time_secs: 2,
    confirmations_required: 12,
    explorer_url: "https://snowtrace.io/tx/{tx}",
};

/// BNB Smart Chain configuration.
pub const BINANCE_SMART_CHAIN: ChainConfig = ChainConfig {
    network: EvmNetwork::BinanceSmartChain,
    chain_id: 56,
    name: "BNB Smart Chain",
    native_symbol: "BNB",
    native_decimals: 18,
    block_time_secs: 3,
    confirmations_required: 15,
    explorer_url: "https://bscscan.com/tx/{tx}",
};

/// zkSync Era configuration.
pub const ZKSYNC: ChainConfig = ChainConfig {
    network: EvmNetwork::ZkSync,
    chain_id: 324,
    name: "zkSync Era",
    native_symbol: "ETH",
    native_decimals: 18,
    block_time_secs: 1,
    confirmations_required: 1, // ZK proofs provide finality
    explorer_url: "https://explorer.zksync.io/tx/{tx}",
};

/// Linea configuration.
pub const LINEA: ChainConfig = ChainConfig {
    network: EvmNetwork::Linea,
    chain_id: 59144,
    name: "Linea",
    native_symbol: "ETH",
    native_decimals: 18,
    block_time_secs: 2,
    confirmations_required: 20,
    explorer_url: "https://lineascan.build/tx/{tx}",
};

/// Scroll configuration.
pub const SCROLL: ChainConfig = ChainConfig {
    network: EvmNetwork::Scroll,
    chain_id: 534352,
    name: "Scroll",
    native_symbol: "ETH",
    native_decimals: 18,
    block_time_secs: 3,
    confirmations_required: 20,
    explorer_url: "https://scrollscan.com/tx/{tx}",
};

/// All supported chains.
pub const ALL_CHAINS: &[ChainConfig] = &[
    ETHEREUM,
    POLYGON,
    ARBITRUM,
    OPTIMISM,
    BASE,
    AVALANCHE,
    BINANCE_SMART_CHAIN,
    ZKSYNC,
    LINEA,
    SCROLL,
];

/// Get chain configuration by network.
pub fn get_chain_config(network: EvmNetwork) -> &'static ChainConfig {
    ALL_CHAINS
        .iter()
        .find(|c| c.network == network)
        .expect("all networks have configs")
}

/// Get chain configuration by chain ID.
pub fn get_chain_config_by_id(chain_id: u64) -> Option<&'static ChainConfig> {
    ALL_CHAINS.iter().find(|c| c.chain_id == chain_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_chain_config() {
        let config = get_chain_config(EvmNetwork::Ethereum);
        assert_eq!(config.chain_id, 1);
        assert_eq!(config.native_symbol, "ETH");
    }

    #[test]
    fn test_get_chain_config_by_id() {
        let config = get_chain_config_by_id(137).unwrap();
        assert_eq!(config.network, EvmNetwork::Polygon);
        assert_eq!(config.native_symbol, "POL");
    }

    #[test]
    fn test_network_from_chain_id() {
        assert_eq!(EvmNetwork::from_chain_id(1), Some(EvmNetwork::Ethereum));
        assert_eq!(EvmNetwork::from_chain_id(137), Some(EvmNetwork::Polygon));
        assert_eq!(EvmNetwork::from_chain_id(999999), None);
    }

    #[test]
    fn test_explorer_url() {
        let tx_url = ETHEREUM.tx_explorer_url("0x123abc");
        assert_eq!(tx_url, "https://etherscan.io/tx/0x123abc");
    }

    #[test]
    fn test_all_chains_have_unique_ids() {
        let mut chain_ids: Vec<u64> = ALL_CHAINS.iter().map(|c| c.chain_id).collect();
        chain_ids.sort();
        chain_ids.dedup();
        assert_eq!(chain_ids.len(), ALL_CHAINS.len());
    }

    #[test]
    fn test_network_chain_id_matches_config() {
        for config in ALL_CHAINS {
            assert_eq!(config.network.chain_id(), config.chain_id);
        }
    }
}
