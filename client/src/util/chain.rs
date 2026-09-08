//! Chain → display-name mapping used throughout the client UI.
//!
//! A UI-layer helper: it picks a short human label for a chain, including
//! testnets. It is not authoritative and cannot be - a CAIP-2 reference is
//! mostly an opaque genesis hash, so no function can turn
//! `monero:418015bb9ae982a1975da7d79277c270` into "Monero". The real mapping is
//! data (`chain_configs`), which is also what lets a plugin name its own chain;
//! this table is the fallback until the client reads that endpoint.
//!
//! Keyed on the full CAIP-2 identifier rather than a number, so a non-EVM chain
//! is nameable here at all.

use types::ChainId;

/// Short human name for a chain.
///
/// Returns the identifier itself for chains not in the table. Deliberately not
/// `"Unknown"`: an opaque identifier is ugly but true, and it lets someone
/// reading the page work out which chain it is.
#[must_use]
pub fn chain_name(chain_id: &ChainId) -> &str {
    match chain_id.as_str() {
        // Mainnets
        "eip155:1" => "Ethereum",
        "eip155:10" => "Optimism",
        "eip155:56" => "BSC",
        "eip155:100" => "Gnosis",
        "eip155:137" => "Polygon",
        "eip155:250" => "Fantom",
        "eip155:324" => "zkSync",
        "eip155:8453" => "Base",
        "eip155:42161" => "Arbitrum",
        "eip155:43114" => "Avalanche",
        "eip155:59144" => "Linea",
        "eip155:534352" => "Scroll",

        // Testnets
        "eip155:97" => "BSC Testnet",
        "eip155:17000" => "Holesky",
        "eip155:43113" => "Fuji",
        "eip155:80002" => "Amoy",
        "eip155:84532" => "Base Sepolia",
        "eip155:421614" => "Arbitrum Sepolia",
        "eip155:560048" => "Hoodi",
        "eip155:11155111" => "Sepolia",
        "eip155:11155420" => "Optimism Sepolia",

        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chain_name_mainnets() {
        assert_eq!(chain_name(&ChainId::evm(1)), "Ethereum");
        assert_eq!(chain_name(&ChainId::evm(137)), "Polygon");
        assert_eq!(chain_name(&ChainId::evm(42161)), "Arbitrum");
        assert_eq!(chain_name(&ChainId::evm(10)), "Optimism");
        assert_eq!(chain_name(&ChainId::evm(8453)), "Base");
        assert_eq!(chain_name(&ChainId::evm(56)), "BSC");
        assert_eq!(chain_name(&ChainId::evm(43114)), "Avalanche");
        assert_eq!(chain_name(&ChainId::evm(324)), "zkSync");
        assert_eq!(chain_name(&ChainId::evm(59144)), "Linea");
        assert_eq!(chain_name(&ChainId::evm(534352)), "Scroll");
        assert_eq!(chain_name(&ChainId::evm(100)), "Gnosis");
        assert_eq!(chain_name(&ChainId::evm(250)), "Fantom");
    }

    #[test]
    fn test_chain_name_testnets() {
        assert_eq!(chain_name(&ChainId::evm(11155111)), "Sepolia");
        assert_eq!(chain_name(&ChainId::evm(17000)), "Holesky");
        assert_eq!(chain_name(&ChainId::evm(84532)), "Base Sepolia");
    }

    #[test]
    fn test_chain_name_unknown() {
        // An unnamed chain renders as its own identifier, not "Unknown". Ugly,
        // but it tells the reader which chain it is - and the real names come
        // from `chain_configs`, which a plugin can populate.
        assert_eq!(chain_name(&ChainId::evm(0)), "eip155:0");
        assert_eq!(chain_name(&ChainId::evm(999_999)), "eip155:999999");
    }

    /// The property the `u64`-keyed version could not have: a non-EVM chain is
    /// nameable here at all.
    #[test]
    fn non_evm_chains_are_expressible() {
        let tron = ChainId::parse("tron:728126428").unwrap();
        assert_eq!(chain_name(&tron), "tron:728126428");

        let monero = ChainId::parse("monero:418015bb9ae982a1975da7d79277c270").unwrap();
        assert_eq!(
            chain_name(&monero),
            "monero:418015bb9ae982a1975da7d79277c270"
        );
    }
}
