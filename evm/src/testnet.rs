//! Testnet chain configurations.
//!
//! This module contains configurations for EVM testnets.
//! Testnets have `network: None` since they don't map to `EvmNetwork` variants.

use crate::network::ChainConfig;

/// Ethereum Sepolia testnet.
pub const SEPOLIA: ChainConfig = ChainConfig {
    network: None,
    chain_id: 11155111,
    name: "Sepolia",
    native_symbol: "ETH",
    native_decimals: 18,
    block_time_secs: 12,
    confirmations_required: 3,
    explorer_url: "https://sepolia.etherscan.io/tx/{tx}",
};

/// Ethereum Holesky testnet.
pub const HOLESKY: ChainConfig = ChainConfig {
    network: None,
    chain_id: 17000,
    name: "Holesky",
    native_symbol: "ETH",
    native_decimals: 18,
    block_time_secs: 12,
    confirmations_required: 3,
    explorer_url: "https://holesky.etherscan.io/tx/{tx}",
};

/// Polygon Amoy testnet.
pub const POLYGON_AMOY: ChainConfig = ChainConfig {
    network: None,
    chain_id: 80002,
    name: "Polygon Amoy",
    native_symbol: "POL",
    native_decimals: 18,
    block_time_secs: 2,
    confirmations_required: 5,
    explorer_url: "https://amoy.polygonscan.com/tx/{tx}",
};

/// Arbitrum Sepolia testnet.
pub const ARBITRUM_SEPOLIA: ChainConfig = ChainConfig {
    network: None,
    chain_id: 421614,
    name: "Arbitrum Sepolia",
    native_symbol: "ETH",
    native_decimals: 18,
    block_time_secs: 1,
    confirmations_required: 5,
    explorer_url: "https://sepolia.arbiscan.io/tx/{tx}",
};

/// Optimism Sepolia testnet.
pub const OPTIMISM_SEPOLIA: ChainConfig = ChainConfig {
    network: None,
    chain_id: 11155420,
    name: "Optimism Sepolia",
    native_symbol: "ETH",
    native_decimals: 18,
    block_time_secs: 2,
    confirmations_required: 5,
    explorer_url: "https://sepolia-optimism.etherscan.io/tx/{tx}",
};

/// Base Sepolia testnet.
pub const BASE_SEPOLIA: ChainConfig = ChainConfig {
    network: None,
    chain_id: 84532,
    name: "Base Sepolia",
    native_symbol: "ETH",
    native_decimals: 18,
    block_time_secs: 2,
    confirmations_required: 5,
    explorer_url: "https://sepolia.basescan.org/tx/{tx}",
};

/// Avalanche Fuji testnet.
pub const AVALANCHE_FUJI: ChainConfig = ChainConfig {
    network: None,
    chain_id: 43113,
    name: "Avalanche Fuji",
    native_symbol: "AVAX",
    native_decimals: 18,
    block_time_secs: 2,
    confirmations_required: 3,
    explorer_url: "https://testnet.snowtrace.io/tx/{tx}",
};

/// BNB Smart Chain testnet.
pub const BSC_TESTNET: ChainConfig = ChainConfig {
    network: None,
    chain_id: 97,
    name: "BSC Testnet",
    native_symbol: "tBNB",
    native_decimals: 18,
    block_time_secs: 3,
    confirmations_required: 5,
    explorer_url: "https://testnet.bscscan.com/tx/{tx}",
};

/// Ethereum Hoodi testnet.
pub const HOODI: ChainConfig = ChainConfig {
    network: None,
    chain_id: 560048,
    name: "Hoodi",
    native_symbol: "ETH",
    native_decimals: 18,
    block_time_secs: 12,
    confirmations_required: 3,
    explorer_url: "https://hoodi.etherscan.io/tx/{tx}",
};

/// All testnet chains.
pub const ALL_TESTNETS: &[ChainConfig] = &[
    SEPOLIA,
    HOLESKY,
    HOODI,
    POLYGON_AMOY,
    ARBITRUM_SEPOLIA,
    OPTIMISM_SEPOLIA,
    BASE_SEPOLIA,
    AVALANCHE_FUJI,
    BSC_TESTNET,
];

/// Get testnet configuration by chain ID.
pub fn get_testnet_config(chain_id: u64) -> Option<&'static ChainConfig> {
    ALL_TESTNETS.iter().find(|c| c.chain_id == chain_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_testnets_have_unique_ids() {
        let mut chain_ids: Vec<u64> = ALL_TESTNETS.iter().map(|c| c.chain_id).collect();
        chain_ids.sort();
        chain_ids.dedup();
        assert_eq!(chain_ids.len(), ALL_TESTNETS.len());
    }

    #[test]
    fn test_testnets_have_no_network() {
        for config in ALL_TESTNETS {
            assert!(config.network.is_none());
        }
    }

    #[test]
    fn test_get_testnet_config() {
        let sepolia = get_testnet_config(11155111).unwrap();
        assert_eq!(sepolia.name, "Sepolia");
        assert!(get_testnet_config(1).is_none()); // Mainnet not a testnet
    }

    /// Every testnet that has USDC seeded in the `tokens` table must be one
    /// this product actually offers - a seed row for a chain id nobody can
    /// select is dead weight at best. Named by id, not iterated from
    /// `ALL_TESTNETS`, so this fails the moment either list drifts from the
    /// other instead of trivially agreeing with itself.
    #[test]
    fn test_seeded_testnets_are_all_offered() {
        for (chain_id, name) in [
            (11155111, "Sepolia"),
            (11155420, "Optimism Sepolia"),
            (421614, "Arbitrum Sepolia"),
            (84532, "Base Sepolia"),
            (43113, "Avalanche Fuji"),
            (80002, "Polygon Amoy"),
        ] {
            let config = get_testnet_config(chain_id)
                .unwrap_or_else(|| panic!("{name} ({chain_id}) is not in ALL_TESTNETS"));
            assert_eq!(config.name, name);
        }
    }
}
