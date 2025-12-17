//! HD wallet derivation for Ethereum addresses.
//!
//! This module implements BIP-32/BIP-44 hierarchical deterministic wallet derivation
//! for generating unique Ethereum addresses. Each invoice gets a unique address
//! derived from a master seed.
//!
//! # Derivation Path
//!
//! We use BIP-44 with purpose 44' and coin type 60' (Ethereum):
//! ```text
//! m / 44' / 60' / account' / change / address_index
//! ```
//!
//! For payment addresses, we use:
//! - account = 0 (default account)
//! - change = 0 (external chain, for receiving)
//! - address_index = incrementing index per invoice

use crate::error::{EvmError, EvmResult};
use alloy::primitives::Address;
use coins_bip32::{path::DerivationPath, prelude::*};
use coins_bip39::{English, Mnemonic};
use std::str::FromStr;

/// Standard BIP-44 coin type for Ethereum.
pub const ETH_COIN_TYPE: u32 = 60;

/// HD wallet for deriving Ethereum addresses.
#[derive(Clone)]
pub struct HdWallet {
    /// The master extended private key.
    master_key: XPriv,
}

impl HdWallet {
    /// Create a new HD wallet from a BIP-39 mnemonic phrase.
    ///
    /// # Arguments
    ///
    /// * `mnemonic` - A valid BIP-39 mnemonic (12, 15, 18, 21, or 24 words)
    /// * `passphrase` - Optional passphrase for additional security (empty string if none)
    ///
    /// # Example
    ///
    /// ```ignore
    /// let wallet = HdWallet::from_mnemonic(
    ///     "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    ///     ""
    /// )?;
    /// ```
    pub fn from_mnemonic(mnemonic: &str, passphrase: &str) -> EvmResult<Self> {
        let mnemonic = Mnemonic::<English>::new_from_phrase(mnemonic)
            .map_err(|e| EvmError::InvalidMnemonic(e.to_string()))?;

        let seed = mnemonic
            .to_seed(Some(passphrase))
            .map_err(|e| EvmError::InvalidMnemonic(format!("failed to derive seed: {}", e)))?;

        let master_key = XPriv::root_from_seed(&seed[..], None)
            .map_err(|e| EvmError::WalletDerivation(e.to_string()))?;

        Ok(Self { master_key })
    }

    /// Create a new HD wallet from raw seed bytes (64 bytes).
    pub fn from_seed(seed: &[u8]) -> EvmResult<Self> {
        if seed.len() != 64 {
            return Err(EvmError::WalletDerivation(format!(
                "seed must be 64 bytes, got {}",
                seed.len()
            )));
        }

        let master_key = XPriv::root_from_seed(seed, None)
            .map_err(|e| EvmError::WalletDerivation(e.to_string()))?;

        Ok(Self { master_key })
    }

    /// Derive an Ethereum address at the given BIP-44 index.
    ///
    /// Uses path: m/44'/60'/0'/0/{index}
    ///
    /// # Arguments
    ///
    /// * `index` - The address index (0, 1, 2, ...)
    ///
    /// # Returns
    ///
    /// The derived Ethereum address as a checksummed string.
    pub fn derive_address(&self, index: u32) -> EvmResult<Address> {
        let path = format!("m/44'/60'/0'/0/{}", index);
        self.derive_address_at_path(&path)
    }

    /// Derive an Ethereum address at a custom derivation path.
    ///
    /// # Arguments
    ///
    /// * `path` - BIP-32 derivation path (e.g., "m/44'/60'/0'/0/0")
    pub fn derive_address_at_path(&self, path: &str) -> EvmResult<Address> {
        let derivation_path = DerivationPath::from_str(path)
            .map_err(|e| EvmError::InvalidDerivationPath(e.to_string()))?;

        let derived_key = self
            .master_key
            .derive_path(&derivation_path)
            .map_err(|e| EvmError::WalletDerivation(e.to_string()))?;

        let public_key = derived_key.verify_key();
        let address = public_key_to_address(&public_key);

        Ok(address)
    }

    /// Derive multiple addresses starting from the given index.
    ///
    /// # Arguments
    ///
    /// * `start_index` - Starting index
    /// * `count` - Number of addresses to derive
    pub fn derive_addresses(&self, start_index: u32, count: u32) -> EvmResult<Vec<Address>> {
        let mut addresses = Vec::with_capacity(count as usize);
        for i in start_index..start_index + count {
            addresses.push(self.derive_address(i)?);
        }
        Ok(addresses)
    }

    /// Get the extended public key for account 0.
    ///
    /// This can be used to derive addresses without the private key.
    pub fn account_xpub(&self) -> EvmResult<XPub> {
        let path = DerivationPath::from_str("m/44'/60'/0'")
            .map_err(|e| EvmError::InvalidDerivationPath(e.to_string()))?;

        let account_key = self
            .master_key
            .derive_path(&path)
            .map_err(|e| EvmError::WalletDerivation(e.to_string()))?;

        Ok(account_key.verify_key())
    }
}

/// Convert a secp256k1 public key to an Ethereum address.
///
/// Ethereum addresses are the last 20 bytes of the Keccak-256 hash
/// of the uncompressed public key (without the 0x04 prefix).
fn public_key_to_address(public_key: &XPub) -> Address {
    use alloy::primitives::keccak256;
    use k256::elliptic_curve::sec1::ToEncodedPoint;
    use k256::PublicKey;

    // Get compressed SEC1 bytes from XPub
    let compressed_bytes = public_key.to_sec1_bytes();

    // Parse as k256 PublicKey and convert to uncompressed format
    let k256_pubkey = PublicKey::from_sec1_bytes(&compressed_bytes)
        .expect("valid public key from XPub");
    let encoded = k256_pubkey.to_encoded_point(false); // false = uncompressed
    let pubkey_bytes = encoded.as_bytes();

    // Skip the 0x04 prefix and hash the 64 remaining bytes
    let hash = keccak256(&pubkey_bytes[1..]);

    // Take the last 20 bytes
    Address::from_slice(&hash[12..])
}

/// Generate a new random mnemonic phrase.
///
/// # Arguments
///
/// * `word_count` - Number of words (12, 15, 18, 21, or 24)
pub fn generate_mnemonic(word_count: usize) -> EvmResult<String> {
    use coins_bip39::Entropy;
    use rand::Rng;

    let entropy_bytes = match word_count {
        12 => 16,
        15 => 20,
        18 => 24,
        21 => 28,
        24 => 32,
        _ => {
            return Err(EvmError::InvalidMnemonic(format!(
                "invalid word count: {}, must be 12, 15, 18, 21, or 24",
                word_count
            )))
        }
    };

    let mut bytes = vec![0u8; entropy_bytes];
    rand::rng().fill(&mut bytes[..]);

    let entropy = Entropy::from_slice(&bytes)
        .map_err(|e| EvmError::InvalidMnemonic(e.to_string()))?;

    let mnemonic = Mnemonic::<English>::new_from_entropy(entropy);

    Ok(mnemonic.to_phrase())
}

/// Validate a mnemonic phrase.
pub fn validate_mnemonic(phrase: &str) -> bool {
    Mnemonic::<English>::new_from_phrase(phrase).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Standard test mnemonic (DO NOT USE IN PRODUCTION)
    const TEST_MNEMONIC: &str =
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    #[test]
    fn test_wallet_from_mnemonic() {
        let wallet = HdWallet::from_mnemonic(TEST_MNEMONIC, "").unwrap();
        let address = wallet.derive_address(0).unwrap();

        // Known address for this mnemonic at m/44'/60'/0'/0/0
        // Compare lowercase since Address debug format doesn't preserve checksum
        assert_eq!(
            format!("{:?}", address).to_lowercase(),
            "0x9858effd232b4033e47d90003d41ec34ecaeda94"
        );
    }

    #[test]
    fn test_derive_multiple_addresses() {
        let wallet = HdWallet::from_mnemonic(TEST_MNEMONIC, "").unwrap();
        let addresses = wallet.derive_addresses(0, 5).unwrap();

        assert_eq!(addresses.len(), 5);

        // Each address should be unique
        let mut unique = addresses.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 5);
    }

    #[test]
    fn test_deterministic_derivation() {
        let wallet1 = HdWallet::from_mnemonic(TEST_MNEMONIC, "").unwrap();
        let wallet2 = HdWallet::from_mnemonic(TEST_MNEMONIC, "").unwrap();

        for i in 0..10 {
            assert_eq!(
                wallet1.derive_address(i).unwrap(),
                wallet2.derive_address(i).unwrap()
            );
        }
    }

    #[test]
    fn test_passphrase_changes_addresses() {
        let wallet1 = HdWallet::from_mnemonic(TEST_MNEMONIC, "").unwrap();
        let wallet2 = HdWallet::from_mnemonic(TEST_MNEMONIC, "secret").unwrap();

        assert_ne!(
            wallet1.derive_address(0).unwrap(),
            wallet2.derive_address(0).unwrap()
        );
    }

    #[test]
    fn test_invalid_mnemonic() {
        let result = HdWallet::from_mnemonic("invalid mnemonic phrase", "");
        assert!(result.is_err());
    }

    #[test]
    fn test_generate_mnemonic() {
        let mnemonic = generate_mnemonic(12).unwrap();
        let words: Vec<&str> = mnemonic.split_whitespace().collect();
        assert_eq!(words.len(), 12);
        assert!(validate_mnemonic(&mnemonic));

        let mnemonic = generate_mnemonic(24).unwrap();
        let words: Vec<&str> = mnemonic.split_whitespace().collect();
        assert_eq!(words.len(), 24);
        assert!(validate_mnemonic(&mnemonic));
    }

    #[test]
    fn test_validate_mnemonic() {
        assert!(validate_mnemonic(TEST_MNEMONIC));
        assert!(!validate_mnemonic("invalid mnemonic"));
        assert!(!validate_mnemonic(""));
    }

    #[test]
    fn test_custom_derivation_path() {
        let wallet = HdWallet::from_mnemonic(TEST_MNEMONIC, "").unwrap();

        // Derive at a custom path
        let addr1 = wallet.derive_address_at_path("m/44'/60'/0'/0/0").unwrap();
        let addr2 = wallet.derive_address(0).unwrap();

        // Should be the same
        assert_eq!(addr1, addr2);
    }
}
