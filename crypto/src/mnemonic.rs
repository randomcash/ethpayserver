//! BIP39 mnemonic generation and recovery key derivation.
//!
//! Uses 24-word (256-bit) mnemonics for account recovery.
//! The mnemonic is passed through Argon2id (in addition to standard PBKDF2)
//! to strengthen the recovery key derivation.

use argon2::{Algorithm, Argon2, Params, Version};
use bip39::{Language, Mnemonic};
use rand::RngCore;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::CryptoError;
use crate::types::SymmetricKey;

/// Recovery mnemonic (24 words, 256 bits of entropy).
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct RecoveryMnemonic {
    phrase: String,
}

impl RecoveryMnemonic {
    /// Generate a new random 24-word mnemonic.
    pub fn generate() -> Result<Self, CryptoError> {
        // Generate 256 bits of entropy for a 24-word mnemonic
        let mut entropy = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut entropy);

        let result = Mnemonic::from_entropy(&entropy)
            .map_err(|e| CryptoError::Mnemonic(format!("Failed to generate mnemonic: {}", e)));

        // Zeroize entropy before returning (success or failure)
        entropy.zeroize();

        Ok(Self {
            phrase: result?.to_string(),
        })
    }

    /// Parse and validate an existing mnemonic phrase.
    pub fn from_phrase(phrase: &str) -> Result<Self, CryptoError> {
        let mnemonic = Mnemonic::parse_normalized(phrase)
            .map_err(|e| CryptoError::Mnemonic(format!("Invalid mnemonic: {}", e)))?;

        // Ensure it's a 24-word mnemonic (256 bits)
        let word_count = mnemonic.word_count();
        if word_count != 24 {
            return Err(CryptoError::Mnemonic(format!(
                "Expected 24-word mnemonic, got {} words",
                word_count
            )));
        }

        Ok(Self {
            phrase: mnemonic.to_string(),
        })
    }

    /// Get the mnemonic phrase (for display to user).
    pub fn phrase(&self) -> &str {
        &self.phrase
    }

    /// Get the words as a vector (for display).
    pub fn words(&self) -> Vec<&str> {
        self.phrase.split_whitespace().collect()
    }

    /// Derive a recovery key from this mnemonic.
    ///
    /// Uses strengthened derivation: BIP39 PBKDF2 + additional Argon2id.
    /// The user_id is used as additional context to bind the key to the account.
    pub fn derive_recovery_key(&self, user_id: &str) -> Result<SymmetricKey, CryptoError> {
        let mnemonic = Mnemonic::parse_normalized(&self.phrase)
            .map_err(|e| CryptoError::Mnemonic(format!("Parse failed: {}", e)))?;

        // Standard BIP39: mnemonic -> seed (uses PBKDF2 with 2048 iterations)
        let mut seed = mnemonic.to_seed("");

        // Additional strengthening with Argon2id
        // This makes brute-forcing the mnemonic much harder
        let salt = format!("payserver-recovery:{}", user_id);

        let params = Params::new(
            65536, // 64 MB memory
            3,     // 3 iterations
            4,     // 4 parallelism
            Some(32),
        )
        .map_err(|e| {
            seed.zeroize();
            CryptoError::Kdf(format!("Invalid Argon2 params: {}", e))
        })?;

        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

        let mut recovery_key = [0u8; 32];
        let result = argon2.hash_password_into(&seed, salt.as_bytes(), &mut recovery_key);

        // Always zeroize the seed (sensitive intermediate material)
        seed.zeroize();

        result.map_err(|e| CryptoError::Kdf(format!("Argon2id failed: {}", e)))?;

        Ok(SymmetricKey(recovery_key))
    }
}

/// Validate a mnemonic phrase without creating the full structure.
pub fn validate_mnemonic(phrase: &str) -> bool {
    Mnemonic::parse_normalized(phrase).is_ok()
}

/// Get the word list for autocomplete/validation.
pub fn word_list() -> &'static [&'static str] {
    Language::English.word_list()
}

/// Check if a word is in the BIP39 word list.
pub fn is_valid_word(word: &str) -> bool {
    Language::English.find_word(word).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_mnemonic() {
        let mnemonic = RecoveryMnemonic::generate().unwrap();
        assert_eq!(mnemonic.words().len(), 24);
    }

    #[test]
    fn test_mnemonic_is_random() {
        let m1 = RecoveryMnemonic::generate().unwrap();
        let m2 = RecoveryMnemonic::generate().unwrap();
        assert_ne!(m1.phrase(), m2.phrase());
    }

    #[test]
    fn test_parse_valid_mnemonic() {
        let generated = RecoveryMnemonic::generate().unwrap();
        let phrase = generated.phrase().to_string();

        let parsed = RecoveryMnemonic::from_phrase(&phrase).unwrap();
        assert_eq!(parsed.phrase(), phrase);
    }

    #[test]
    fn test_parse_invalid_mnemonic() {
        let result = RecoveryMnemonic::from_phrase("invalid mnemonic phrase");
        assert!(result.is_err());
    }

    #[test]
    fn test_reject_12_word_mnemonic() {
        // Valid 12-word mnemonic
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let result = RecoveryMnemonic::from_phrase(phrase);
        assert!(matches!(result, Err(CryptoError::Mnemonic(_))));
    }

    #[test]
    fn test_derive_recovery_key() {
        let mnemonic = RecoveryMnemonic::generate().unwrap();
        let key = mnemonic.derive_recovery_key("user123").unwrap();
        assert_eq!(key.as_bytes().len(), 32);
    }

    #[test]
    fn test_recovery_key_deterministic() {
        let mnemonic = RecoveryMnemonic::generate().unwrap();
        let phrase = mnemonic.phrase().to_string();

        let m1 = RecoveryMnemonic::from_phrase(&phrase).unwrap();
        let m2 = RecoveryMnemonic::from_phrase(&phrase).unwrap();

        let key1 = m1.derive_recovery_key("user123").unwrap();
        let key2 = m2.derive_recovery_key("user123").unwrap();

        assert_eq!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn test_recovery_key_differs_by_user() {
        let mnemonic = RecoveryMnemonic::generate().unwrap();
        let phrase = mnemonic.phrase().to_string();

        let m1 = RecoveryMnemonic::from_phrase(&phrase).unwrap();
        let m2 = RecoveryMnemonic::from_phrase(&phrase).unwrap();

        let key1 = m1.derive_recovery_key("user123").unwrap();
        let key2 = m2.derive_recovery_key("user456").unwrap();

        assert_ne!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn test_validate_mnemonic() {
        let mnemonic = RecoveryMnemonic::generate().unwrap();
        assert!(validate_mnemonic(mnemonic.phrase()));
        assert!(!validate_mnemonic("not a valid mnemonic"));
    }

    #[test]
    fn test_is_valid_word() {
        assert!(is_valid_word("abandon"));
        assert!(is_valid_word("zoo"));
        assert!(!is_valid_word("notaword"));
    }

    #[test]
    fn test_word_list_not_empty() {
        let words = word_list();
        assert_eq!(words.len(), 2048);
    }
}
