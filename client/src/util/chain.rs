//! Chain-ID → display-name mapping used throughout the client UI.
//!
//! This is a UI-layer helper: it picks a short human label for a chain ID,
//! including testnets. The authoritative `Network` enum lives in
//! `payserver-commons/types` but (a) it has no `from_chain_id()` yet and
//! (b) doesn't include testnets. Until both land there (tracked as a
//! separate follow-up), this keeps the mapping in one place client-side
//! instead of inline in every page.

/// Short human name for an EIP-155 chain ID.
///
/// Returns `"Unknown"` for chain IDs not in the table.
#[must_use]
pub fn chain_name(chain_id: u64) -> &'static str {
    match chain_id {
        // Mainnets
        1 => "Ethereum",
        10 => "Optimism",
        56 => "BSC",
        100 => "Gnosis",
        137 => "Polygon",
        250 => "Fantom",
        324 => "zkSync",
        8453 => "Base",
        42161 => "Arbitrum",
        43114 => "Avalanche",
        59144 => "Linea",
        534352 => "Scroll",

        // Testnets
        97 => "BSC Testnet",
        17000 => "Holesky",
        43113 => "Fuji",
        80002 => "Amoy",
        84532 => "Base Sepolia",
        421614 => "Arbitrum Sepolia",
        560048 => "Hoodi",
        11155111 => "Sepolia",
        11155420 => "Optimism Sepolia",

        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chain_name_mainnets() {
        assert_eq!(chain_name(1), "Ethereum");
        assert_eq!(chain_name(137), "Polygon");
        assert_eq!(chain_name(42161), "Arbitrum");
        assert_eq!(chain_name(10), "Optimism");
        assert_eq!(chain_name(8453), "Base");
        assert_eq!(chain_name(56), "BSC");
        assert_eq!(chain_name(43114), "Avalanche");
        assert_eq!(chain_name(324), "zkSync");
        assert_eq!(chain_name(59144), "Linea");
        assert_eq!(chain_name(534352), "Scroll");
        assert_eq!(chain_name(100), "Gnosis");
        assert_eq!(chain_name(250), "Fantom");
    }

    #[test]
    fn test_chain_name_testnets() {
        assert_eq!(chain_name(11155111), "Sepolia");
        assert_eq!(chain_name(17000), "Holesky");
        assert_eq!(chain_name(84532), "Base Sepolia");
    }

    #[test]
    fn test_chain_name_unknown() {
        assert_eq!(chain_name(0), "Unknown");
        assert_eq!(chain_name(999_999), "Unknown");
    }
}
