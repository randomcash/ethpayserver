//! Key Derivation Functions: Argon2id and HKDF-SHA256.
//!
//! Following Bitwarden's key derivation model:
//! 1. Password + Email -> Argon2id -> Master Key (256-bit)
//! 2. Master Key -> HKDF-Expand -> Stretched Key (512-bit = enc + mac)
//! 3. Master Key + Password -> PBKDF2 (1 iter) -> Master Password Hash

use argon2::{Algorithm, Argon2, Params, Version};
use hkdf::Hkdf;
use pbkdf2::pbkdf2_hmac;
use sha2::Sha256;
use zeroize::Zeroize;

use crate::error::CryptoError;
use crate::types::{KdfParams, MasterPasswordHash, StretchedKey, SymmetricKey};

/// Derive the Master Key from password and email using Argon2id.
///
/// # Arguments
/// * `password` - User's password
/// * `email` - User's email (used as salt base)
/// * `params` - KDF parameters (memory, iterations, parallelism, salt)
///
/// # Returns
/// A 256-bit Master Key
pub fn derive_master_key(
    password: &[u8],
    email: &str,
    params: &KdfParams,
) -> Result<SymmetricKey, CryptoError> {
    // Combine email with random salt for the final salt
    let mut salt = email.to_lowercase().as_bytes().to_vec();
    salt.extend_from_slice(&params.salt);

    let argon2_params = Params::new(
        params.memory_kb,
        params.iterations,
        params.parallelism,
        Some(32),
    )
    .map_err(|e| CryptoError::Kdf(format!("Invalid Argon2 params: {}", e)))?;

    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon2_params);

    let mut master_key = [0u8; 32];
    argon2
        .hash_password_into(password, &salt, &mut master_key)
        .map_err(|e| CryptoError::Kdf(format!("Argon2id failed: {}", e)))?;

    Ok(SymmetricKey(master_key))
}

/// Stretch the Master Key to 512 bits using HKDF-SHA256.
///
/// Returns a StretchedKey containing:
/// - enc_key (256-bit): For AES-256-CBC encryption
/// - mac_key (256-bit): For HMAC-SHA256 authentication
pub fn stretch_master_key(master_key: &SymmetricKey) -> Result<StretchedKey, CryptoError> {
    let hkdf = Hkdf::<Sha256>::new(Some(master_key.as_bytes()), master_key.as_bytes());

    let mut okm = [0u8; 64];
    let result = (|| {
        hkdf.expand(b"enc", &mut okm[..32])
            .map_err(|_| CryptoError::Kdf("HKDF expand failed for enc key".into()))?;
        hkdf.expand(b"mac", &mut okm[32..])
            .map_err(|_| CryptoError::Kdf("HKDF expand failed for mac key".into()))?;
        Ok(StretchedKey::from_bytes(&okm))
    })();

    // Always zeroize intermediate key material
    okm.zeroize();
    result
}

/// Derive the Master Password Hash for server-side authentication.
///
/// Uses PBKDF2-SHA256 with 1 iteration (the Master Key already has
/// strong KDF applied, this is just for creating a verifiable hash).
pub fn derive_master_password_hash(
    master_key: &SymmetricKey,
    password: &[u8],
) -> MasterPasswordHash {
    let mut hash = [0u8; 32];
    pbkdf2_hmac::<Sha256>(master_key.as_bytes(), password, 1, &mut hash);
    MasterPasswordHash(hash)
}

/// Derive a key from arbitrary data using HKDF-SHA256.
///
/// Useful for deriving sub-keys from a master key.
pub fn hkdf_derive(
    ikm: &[u8],
    salt: Option<&[u8]>,
    info: &[u8],
    output_len: usize,
) -> Result<Vec<u8>, CryptoError> {
    let hkdf = Hkdf::<Sha256>::new(salt, ikm);
    let mut okm = vec![0u8; output_len];
    hkdf.expand(info, &mut okm)
        .map_err(|_| CryptoError::Kdf("HKDF expand failed".into()))?;
    Ok(okm)
}

/// Generate random bytes.
pub fn random_bytes(len: usize) -> Vec<u8> {
    use rand::RngCore;
    let mut bytes = vec![0u8; len];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
}

/// Generate a random 256-bit symmetric key.
pub fn generate_symmetric_key() -> SymmetricKey {
    use rand::RngCore;
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    SymmetricKey(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_master_key() {
        let params = KdfParams::new_random();
        let master_key = derive_master_key(b"password123", "test@example.com", &params).unwrap();

        assert_eq!(master_key.as_bytes().len(), 32);

        // Same inputs should produce same output
        let master_key2 = derive_master_key(b"password123", "test@example.com", &params).unwrap();
        assert_eq!(master_key.as_bytes(), master_key2.as_bytes());

        // Different password should produce different output
        let master_key3 = derive_master_key(b"different", "test@example.com", &params).unwrap();
        assert_ne!(master_key.as_bytes(), master_key3.as_bytes());
    }

    #[test]
    fn test_stretch_master_key() {
        let master_key = SymmetricKey([42u8; 32]);
        let stretched = stretch_master_key(&master_key).unwrap();

        assert_eq!(stretched.enc_key.len(), 32);
        assert_eq!(stretched.mac_key.len(), 32);

        // enc_key and mac_key should be different
        assert_ne!(stretched.enc_key, stretched.mac_key);

        // Should be deterministic
        let stretched2 = stretch_master_key(&master_key).unwrap();
        assert_eq!(stretched.enc_key, stretched2.enc_key);
        assert_eq!(stretched.mac_key, stretched2.mac_key);
    }

    #[test]
    fn test_derive_master_password_hash() {
        let master_key = SymmetricKey([42u8; 32]);
        let hash = derive_master_password_hash(&master_key, b"password123");

        assert_eq!(hash.as_bytes().len(), 32);

        // Should be deterministic
        let hash2 = derive_master_password_hash(&master_key, b"password123");
        assert_eq!(hash.as_bytes(), hash2.as_bytes());

        // Different password should produce different hash
        let hash3 = derive_master_password_hash(&master_key, b"different");
        assert_ne!(hash.as_bytes(), hash3.as_bytes());
    }

    #[test]
    fn test_hkdf_derive() {
        let ikm = b"input key material";
        let salt = b"salt";
        let info = b"context info";

        let key1 = hkdf_derive(ikm, Some(salt), info, 32).unwrap();
        assert_eq!(key1.len(), 32);

        // Should be deterministic
        let key2 = hkdf_derive(ikm, Some(salt), info, 32).unwrap();
        assert_eq!(key1, key2);

        // Different info should produce different key
        let key3 = hkdf_derive(ikm, Some(salt), b"different", 32).unwrap();
        assert_ne!(key1, key3);
    }

    #[test]
    fn test_generate_symmetric_key() {
        let key1 = generate_symmetric_key();
        let key2 = generate_symmetric_key();

        assert_eq!(key1.as_bytes().len(), 32);
        assert_ne!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn test_random_bytes() {
        let bytes1 = random_bytes(32);
        let bytes2 = random_bytes(32);

        assert_eq!(bytes1.len(), 32);
        assert_ne!(bytes1, bytes2);
    }
}
