//! HD wallet derivation for secp256k1 receiving addresses.
//!
//! This module implements BIP-32/BIP-44 hierarchical deterministic wallet derivation
//! for generating a unique receiving address per invoice.
//!
//! # Derivation Path
//!
//! BIP-44 with purpose 44' and the coin type of the key's chain family - 60'
//! for Ethereum, 195' for Tron (see [`crate::family`]):
//! ```text
//! m / 44' / coin' / account' / change / address_index
//! ```
//!
//! For payment addresses, we use:
//! - account = 0 (default account)
//! - change = 0 (external chain, for receiving)
//! - address_index = incrementing index per invoice
//!
//! The coin type is the load-bearing part, and it is *not* recoverable from
//! the account-level xpub a merchant registers. Everything here therefore
//! takes the family explicitly rather than assuming Ethereum - an assumption
//! that, applied to a Tron key, derives addresses the merchant's own wallet
//! will never show.

use crate::error::{EvmError, EvmResult};
use crate::family::ChainFamily;
use alloy::primitives::Address;
use coins_bip32::{
    enc::{MainnetEncoder, XKeyEncoder},
    path::DerivationPath,
    prelude::*,
};
use coins_bip39::{English, Mnemonic};
use std::str::FromStr;

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

        // `root_from_seed(_, None)` defaults to `Hint::SegWit`, which would
        // render an account xpub with BIP-84 ("zpub") version bytes instead
        // of the BIP-44 ("xpub") ones every merchant wallet and this crate's
        // own docs mean by "xpub". The hint changes nothing about derivation
        // - addresses come out identical either way - only how a key
        // encodes to base58, so this is spelled out explicitly rather than
        // left to the library default.
        let master_key = XPriv::root_from_seed(&seed[..], Some(Hint::Legacy))
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

        let master_key = XPriv::root_from_seed(seed, Some(Hint::Legacy))
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
        self.derive_address_for(ChainFamily::Evm, index)
    }

    /// Derive the receiving address bytes for one chain family at `index`.
    ///
    /// The family chooses the coin type, so the same seed produces entirely
    /// different keys - and therefore entirely different addresses - for
    /// Ethereum and for Tron. That is the property that makes the two
    /// non-interchangeable, and re-encoding one family's address in the
    /// other's alphabet does not produce the other's address.
    ///
    /// Returns raw address bytes; render them with
    /// [`ChainFamily::encode_address`].
    pub fn derive_address_for(&self, family: ChainFamily, index: u32) -> EvmResult<Address> {
        self.derive_address_at_path(&family.derivation_path(index))
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

    /// Derive the private key at the given BIP-44 index.
    ///
    /// Uses path: m/44'/60'/0'/0/{index}
    ///
    /// # Security
    ///
    /// The returned key bytes must be handled securely and zeroized after use.
    /// Never log or persist private keys.
    pub fn derive_private_key(&self, index: u32) -> EvmResult<[u8; 32]> {
        let path = format!("m/44'/60'/0'/0/{}", index);
        let derivation_path = DerivationPath::from_str(&path)
            .map_err(|e| EvmError::InvalidDerivationPath(e.to_string()))?;

        let derived_key = self
            .master_key
            .derive_path(&derivation_path)
            .map_err(|e| EvmError::WalletDerivation(e.to_string()))?;

        let signing_key: &k256::ecdsa::SigningKey = derived_key.as_ref();
        let key_bytes = signing_key.to_bytes();
        let mut result = [0u8; 32];
        result.copy_from_slice(&key_bytes);
        Ok(result)
    }

    /// Get the extended public key for account 0.
    ///
    /// This can be used to derive addresses without the private key.
    pub fn account_xpub(&self) -> EvmResult<XPub> {
        self.account_xpub_for(ChainFamily::Evm)
    }

    /// The account-level extended public key a merchant's wallet would export
    /// for one chain family - `m/44'/60'/0'` or `m/44'/195'/0'`.
    ///
    /// The two are byte-indistinguishable in form: same version bytes, same
    /// base58 alphabet, same length. Only the key material differs, and
    /// nothing can read the coin type back out of it. That is why the family
    /// is stored beside the key rather than inferred from it.
    pub fn account_xpub_for(&self, family: ChainFamily) -> EvmResult<XPub> {
        let path = DerivationPath::from_str(&family.account_path())
            .map_err(|e| EvmError::InvalidDerivationPath(e.to_string()))?;

        let account_key = self
            .master_key
            .derive_path(&path)
            .map_err(|e| EvmError::WalletDerivation(e.to_string()))?;

        Ok(account_key.verify_key())
    }

    /// [`Self::account_xpub_for`], base58-encoded - the exact string a
    /// merchant pastes into `POST /wallets`.
    pub fn account_xpub_string_for(&self, family: ChainFamily) -> EvmResult<String> {
        MainnetEncoder::xpub_to_base58(&self.account_xpub_for(family)?)
            .map_err(|e| EvmError::WalletDerivation(e.to_string()))
    }
}

/// Convert a secp256k1 public key to an Ethereum address.
///
/// Ethereum addresses are the last 20 bytes of the Keccak-256 hash
/// of the uncompressed public key (without the 0x04 prefix).
fn public_key_to_address(public_key: &XPub) -> Address {
    use alloy::primitives::keccak256;
    use k256::PublicKey;
    use k256::elliptic_curve::sec1::ToEncodedPoint;

    // Get compressed SEC1 bytes from XPub
    let compressed_bytes = public_key.to_sec1_bytes();

    // Parse as k256 PublicKey and convert to uncompressed format
    let k256_pubkey =
        PublicKey::from_sec1_bytes(&compressed_bytes).expect("valid public key from XPub");
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
            )));
        }
    };

    let mut bytes = vec![0u8; entropy_bytes];
    rand::rng().fill(&mut bytes[..]);

    let entropy =
        Entropy::from_slice(&bytes).map_err(|e| EvmError::InvalidMnemonic(e.to_string()))?;

    let mnemonic = Mnemonic::<English>::new_from_entropy(entropy);

    Ok(mnemonic.to_phrase())
}

/// Validate a mnemonic phrase.
pub fn validate_mnemonic(phrase: &str) -> bool {
    Mnemonic::<English>::new_from_phrase(phrase).is_ok()
}

/// Address deriver from an extended public key, bound to a chain family.
///
/// Allows deriving payment addresses without access to private keys.
/// Merchants provide their xpub and the server derives unique addresses
/// for each invoice.
///
/// The family is carried, not guessed. An account xpub has its coin type
/// already spent - the deriver only walks `0/{index}` below it - so by the
/// time a key reaches here, which family it belongs to is no longer a
/// question this code could answer from the bytes. Every constructor
/// therefore demands it, and [`Self::derive_evm_address`] refuses to hand out
/// EVM-typed bytes for a key that was not registered as EVM.
#[derive(Clone)]
pub struct XpubDeriver {
    /// The account-level extended public key (at path m/44'/{coin}'/0')
    account_xpub: XPub,
    /// The family the key was registered for. Decides how an address is
    /// rendered, and which callers may use it at all.
    family: ChainFamily,
}

impl XpubDeriver {
    /// Create a deriver for a key registered under `namespace`.
    ///
    /// The namespace comes from the wallet row the key was read from - never
    /// from the key, which cannot be asked, and never from a default. An
    /// unknown namespace is an error rather than a fallback to Ethereum.
    pub fn from_xpub(namespace: &str, xpub_str: &str) -> EvmResult<Self> {
        Self::for_family(crate::family::family_for_namespace(namespace)?, xpub_str)
    }

    /// Create a deriver for a known family.
    pub fn for_family(family: ChainFamily, xpub_str: &str) -> EvmResult<Self> {
        let account_xpub = MainnetEncoder::xpub_from_base58(xpub_str)
            .map_err(|e| EvmError::InvalidXpub(format!("failed to parse xpub: {}", e)))?;

        Ok(Self {
            account_xpub,
            family,
        })
    }

    /// The family this key was registered for.
    pub fn family(&self) -> ChainFamily {
        self.family
    }

    /// The full BIP-44 path of the address at `index`, coin type included.
    pub fn derivation_path(&self, index: u32) -> String {
        self.family.derivation_path(index)
    }

    /// Derive the receiving address at `index`, rendered in this key's own
    /// family encoding.
    ///
    /// `0x…` for `eip155`, `T…` for `tron`. This is what a merchant compares
    /// against their own wallet, and what a customer is asked to pay.
    pub fn derive_address(&self, index: u32) -> EvmResult<String> {
        Ok(self
            .family
            .encode_address(self.derive_address_bytes(index)?))
    }

    /// Derive the address at `index` as EVM address bytes.
    ///
    /// Refuses a key registered for any other family. The bytes are the same
    /// shape whatever the family, which is exactly why this refuses rather
    /// than converts: an `Address` flows on into the chain monitor and the
    /// watched-address table, both of which mean *an EVM address on an EVM
    /// chain*, and a Tron key's bytes there would have the server watching an
    /// Ethereum address for a payment that is never coming.
    pub fn derive_evm_address(&self, index: u32) -> EvmResult<Address> {
        if self.family != ChainFamily::Evm {
            return Err(EvmError::InvalidXpub(format!(
                "this key is registered for `{}`, not an EVM chain; it cannot                  derive an EVM address",
                self.family.namespace()
            )));
        }
        self.derive_address_bytes(index)
    }

    /// Walk `0/{index}` below the account key. Family-independent: the two
    /// families differ in the coin type already spent above this point, and
    /// in nothing below it.
    fn derive_address_bytes(&self, index: u32) -> EvmResult<Address> {
        // Derive external chain (0) then index
        let external = self
            .account_xpub
            .derive_child(0)
            .map_err(|e| EvmError::WalletDerivation(e.to_string()))?;

        let derived = external
            .derive_child(index)
            .map_err(|e| EvmError::WalletDerivation(e.to_string()))?;

        Ok(public_key_to_address(&derived))
    }

    /// Get the xpub as a base58-encoded string.
    pub fn xpub_string(&self) -> EvmResult<String> {
        MainnetEncoder::xpub_to_base58(&self.account_xpub)
            .map_err(|e| EvmError::WalletDerivation(e.to_string()))
    }
}

/// Validate an extended public key string.
pub fn validate_xpub(xpub_str: &str) -> bool {
    if xpub_str.is_empty() || xpub_str.len() < 10 {
        return false;
    }
    MainnetEncoder::xpub_from_base58(xpub_str).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Standard test mnemonic (DO NOT USE IN PRODUCTION)
    const TEST_MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    /// What a wallet exports for `TEST_MNEMONIC` at `m/44'/60'/0'` - the
    /// string a merchant pastes to register an Ethereum receiving key.
    const EVM_ACCOUNT_XPUB: &str = "xpub6DCoCpSuQZB2jawqnGMEPS63ePKWkwWPH4TU45Q7LPXWuNd8TMtVxRrgjtEshuqpK3mdhaWHPFsBngh5GFZaM6si3yZdUsT8ddYM3PwnATt";

    /// The same seed at `m/44'/195'/0'` - Tron. Note that nothing about the
    /// two strings says which is which.
    const TRON_ACCOUNT_XPUB: &str = "xpub6D1AabNHCupeiLM65ZR9UStMhJ1vCpyV4XbZdyhMZBiJXALQtmn9p42VTQckoHVn8WNqS7dqnJokZHAHcHGoaQgmv8D45oNUKx6DZMNZBCd";

    /// `HdWallet` must export the exact string a real wallet would - not just
    /// something `XpubDeriver` can parse back.
    ///
    /// Every other test that touches `EVM_ACCOUNT_XPUB`/`TRON_ACCOUNT_XPUB`
    /// hands the literal to `XpubDeriver::from_xpub`, which round-trips fine
    /// no matter which BIP-32/49/84 encoding produced the bytes - the base58
    /// version prefix only picks which of `xpub`/`ypub`/`zpub` comes out, and
    /// parsing recovers it either way. That let `HdWallet::from_mnemonic`
    /// default to the underlying library's `Hint::SegWit` and export `zpub…`
    /// silently: every derived address stayed correct, `validate_xpub` still
    /// accepted the result, and nothing here caught that the string handed to
    /// a merchant no longer looked anything like the `xpub…` this repo's own
    /// docs and API examples promise.
    #[test]
    fn account_xpub_string_matches_what_a_real_wallet_exports() {
        let wallet = HdWallet::from_mnemonic(TEST_MNEMONIC, "").unwrap();
        assert_eq!(
            wallet.account_xpub_string_for(ChainFamily::Evm).unwrap(),
            EVM_ACCOUNT_XPUB
        );
        assert_eq!(
            wallet.account_xpub_string_for(ChainFamily::Tron).unwrap(),
            TRON_ACCOUNT_XPUB
        );
    }

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

    #[test]
    fn test_xpub_deriver_matches_wallet() {
        let wallet = HdWallet::from_mnemonic(TEST_MNEMONIC, "").unwrap();
        let xpub = wallet.account_xpub().unwrap();
        let xpub_str = MainnetEncoder::xpub_to_base58(&xpub).unwrap();
        let deriver = XpubDeriver::from_xpub("eip155", &xpub_str).unwrap();

        // XpubDeriver should derive the same addresses as HdWallet
        for i in 0..5 {
            assert_eq!(
                wallet.derive_address(i).unwrap(),
                deriver.derive_evm_address(i).unwrap()
            );
        }
    }

    /// The coin type is honoured, not merely the encoding swapped.
    ///
    /// This is the assertion the whole family-scoping change rests on, and the
    /// one that a plausible-looking wrong implementation fails. Tron addresses
    /// are base58check over the same 20 bytes Ethereum renders as hex, so it
    /// is entirely possible to "add Tron support" by re-encoding an Ethereum
    /// key and produce valid, checksum-correct `T…` addresses for every index.
    /// They would be addresses at `m/44'/60'`, and the merchant's Tron wallet
    /// looks under `m/44'/195'` - so the money arrives somewhere only a seed
    /// re-import at a non-standard path can reach.
    ///
    /// So: from one seed, the Tron address must not be the Tron encoding of
    /// the Ethereum address. It must be a different key entirely.
    #[test]
    fn a_tron_address_is_not_the_evm_address_in_another_alphabet() {
        use crate::family::ChainFamily;

        let wallet = HdWallet::from_mnemonic(TEST_MNEMONIC, "").unwrap();

        for index in 0..3u32 {
            let evm_bytes = wallet.derive_address_for(ChainFamily::Evm, index).unwrap();
            let tron_bytes = wallet.derive_address_for(ChainFamily::Tron, index).unwrap();

            assert_ne!(
                evm_bytes, tron_bytes,
                "index {index}: m/44'/60'/0'/0/{index} and m/44'/195'/0'/0/{index}                  produced the same key - the coin type is being ignored"
            );

            // And stated the way the bug would actually appear: the Tron
            // address is not the EVM address wearing base58.
            assert_ne!(
                ChainFamily::Tron.encode_address(tron_bytes),
                ChainFamily::Tron.encode_address(evm_bytes),
                "index {index}: the Tron address is just the EVM key re-encoded"
            );
        }
    }

    /// Both families' addresses for one seed, against an independent
    /// derivation of the standard BIP-39 test mnemonic.
    ///
    /// Published values, not values this code produced: an implementation that
    /// is self-consistently wrong about the coin type passes every relative
    /// assertion in this file, and only a fixed external vector catches it.
    /// The Ethereum value is the one this module has always asserted; the Tron
    /// values are at `m/44'/195'/0'/0/{0,1,2}`.
    #[test]
    fn one_seed_derives_the_published_addresses_for_each_family() {
        use crate::family::ChainFamily;

        let wallet = HdWallet::from_mnemonic(TEST_MNEMONIC, "").unwrap();

        let evm: Vec<String> = (0..3)
            .map(|i| {
                ChainFamily::Evm
                    .encode_address(wallet.derive_address_for(ChainFamily::Evm, i).unwrap())
            })
            .collect();
        assert_eq!(
            evm[0].to_lowercase(),
            "0x9858effd232b4033e47d90003d41ec34ecaeda94"
        );

        let tron: Vec<String> = (0..3)
            .map(|i| {
                ChainFamily::Tron
                    .encode_address(wallet.derive_address_for(ChainFamily::Tron, i).unwrap())
            })
            .collect();
        assert_eq!(
            tron,
            vec![
                "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH".to_string(),
                "TSeJkUh4Qv67VNFwY8LaAxERygNdy6NQZK".to_string(),
                "TYJPRrdB5APNeRs4R7fYZSwW3TcrTKw2gx".to_string(),
            ]
        );
    }

    /// The two families' account xpubs are indistinguishable by inspection.
    ///
    /// The premise of the whole design. If a server could tell them apart it
    /// would not need to ask, and every merchant-facing warning here would be
    /// unnecessary ceremony. It cannot: same version bytes, same alphabet,
    /// same length, both accepted by the only check a merchant's paste is
    /// subjected to.
    ///
    /// These are the strings a merchant actually pastes - what a wallet
    /// exports for the standard BIP-39 test mnemonic at `m/44'/60'/0'` and
    /// `m/44'/195'/0'` - rather than what this crate's encoder emits, which is
    /// a `zpub` and so would hide the very similarity being asserted.
    #[test]
    fn an_ethereum_xpub_and_a_tron_xpub_cannot_be_told_apart() {
        assert_ne!(
            EVM_ACCOUNT_XPUB, TRON_ACCOUNT_XPUB,
            "different keys, or the coin type did nothing"
        );
        assert_eq!(EVM_ACCOUNT_XPUB.len(), TRON_ACCOUNT_XPUB.len());
        assert!(EVM_ACCOUNT_XPUB.starts_with("xpub") && TRON_ACCOUNT_XPUB.starts_with("xpub"));
        assert!(
            validate_xpub(EVM_ACCOUNT_XPUB) && validate_xpub(TRON_ACCOUNT_XPUB),
            "both pass the only check a merchant's paste is subjected to"
        );
    }

    /// Registering a key by the string a merchant pastes produces the
    /// published addresses for its family - and, for the same seed, the two
    /// families produce different ones.
    ///
    /// This is the entry point that matters: `POST /wallets` hands an xpub
    /// string and a namespace to exactly this pair of calls. The Ethereum
    /// value is the one this module has asserted since it was written; the
    /// Tron values come from an independent derivation at `m/44'/195'/0'/0/i`.
    #[test]
    fn a_registered_xpub_derives_its_own_family_published_addresses() {
        let evm = XpubDeriver::from_xpub("eip155", EVM_ACCOUNT_XPUB).unwrap();
        assert_eq!(
            evm.derive_address(0).unwrap().to_lowercase(),
            "0x9858effd232b4033e47d90003d41ec34ecaeda94"
        );

        let tron = XpubDeriver::from_xpub("tron", TRON_ACCOUNT_XPUB).unwrap();
        let first_three: Vec<String> = (0..3).map(|i| tron.derive_address(i).unwrap()).collect();
        assert_eq!(
            first_three,
            vec![
                "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH".to_string(),
                "TSeJkUh4Qv67VNFwY8LaAxERygNdy6NQZK".to_string(),
                "TYJPRrdB5APNeRs4R7fYZSwW3TcrTKw2gx".to_string(),
            ]
        );

        // The Ethereum key run through Tron's encoding is a valid `T…`
        // address, and it is not any of those. That is the failure mode: it
        // looks right, it passes every checksum, and the merchant's Tron
        // wallet never shows it.
        let misfiled = XpubDeriver::from_xpub("tron", EVM_ACCOUNT_XPUB).unwrap();
        let wrong = misfiled.derive_address(0).unwrap();
        assert!(wrong.starts_with('T'));
        assert!(!first_three.contains(&wrong));
    }

    /// A Tron key refuses to be used where EVM address bytes are expected.
    ///
    /// `derive_evm_address` feeds the chain monitor and the watched-address
    /// table, which mean an address on an EVM chain. Bytes from a Tron key put
    /// there would have the server watching an Ethereum address nobody will
    /// ever pay.
    #[test]
    fn a_tron_key_cannot_be_used_as_an_evm_key() {
        use crate::family::ChainFamily;

        let wallet = HdWallet::from_mnemonic(TEST_MNEMONIC, "").unwrap();
        let xpub =
            MainnetEncoder::xpub_to_base58(&wallet.account_xpub_for(ChainFamily::Tron).unwrap())
                .unwrap();

        let deriver = XpubDeriver::from_xpub("tron", &xpub).unwrap();
        assert!(deriver.derive_evm_address(0).is_err());
        assert!(deriver.derive_address(0).unwrap().starts_with('T'));
        assert_eq!(deriver.derivation_path(0), "m/44'/195'/0'/0/0");

        // And a namespace with no derivation at all is refused outright,
        // rather than quietly treated as Ethereum.
        assert!(XpubDeriver::from_xpub("solana", &xpub).is_err());
    }

    #[test]
    fn test_derive_private_key() {
        let wallet = HdWallet::from_mnemonic(TEST_MNEMONIC, "").unwrap();
        let key = wallet.derive_private_key(0).unwrap();

        // Should be 32 bytes
        assert_eq!(key.len(), 32);

        // Should be deterministic
        let key2 = wallet.derive_private_key(0).unwrap();
        assert_eq!(key, key2);

        // Different indexes should produce different keys
        let key3 = wallet.derive_private_key(1).unwrap();
        assert_ne!(key, key3);
    }

    #[test]
    fn test_xpub_validation() {
        let wallet = HdWallet::from_mnemonic(TEST_MNEMONIC, "").unwrap();
        let xpub = wallet.account_xpub().unwrap();
        let xpub_str = MainnetEncoder::xpub_to_base58(&xpub).unwrap();

        assert!(validate_xpub(&xpub_str));
        assert!(!validate_xpub("invalid-xpub"));
        assert!(!validate_xpub(""));
    }

    /// A private key must never be accepted where a public one is asked for.
    ///
    /// The whole custody story rests on this: a merchant hands over an xpub, the
    /// server derives receive addresses from it, and nothing here can move their
    /// funds because nothing here can sign. An `xprv` pasted into the same
    /// field, by a merchant who does not know the difference or who copied the
    /// wrong line out of their wallet, would put a spending key in our database
    /// and silently make us custodial.
    ///
    /// `xpub_from_base58` rejects it on the version-byte prefix (0x0488ADE4 for
    /// private, 0x0488B21E for public). That is a property of the decoder rather
    /// than a check anyone wrote here, which is exactly why it deserves a test:
    /// nothing in this file would notice if it stopped being true.
    #[test]
    fn an_xprv_is_never_accepted_as_an_xpub() {
        // BIP-32 test vector 1 master keys - a real pair, same seed.
        const XPRV: &str = "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi";
        const XPUB: &str = "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet8";

        assert!(validate_xpub(XPUB), "a valid xpub must be accepted");
        assert!(
            !validate_xpub(XPRV),
            "an xprv was accepted as an xpub: the server would be holding a spending key"
        );
    }
}
