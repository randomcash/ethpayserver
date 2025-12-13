//! Symmetric encryption: AES-256-CBC + HMAC-SHA256 (Encrypt-then-MAC).
//!
//! Following Bitwarden's encryption model:
//! - AES-256-CBC for encryption
//! - HMAC-SHA256 for authentication (computed over IV || ciphertext)
//! - Random 128-bit IV for each encryption

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use zeroize::Zeroize;

use crate::error::CryptoError;
use crate::kdf::random_bytes;
use crate::types::{EncryptedBlob, StretchedKey, SymmetricKey};

type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;
type HmacSha256 = Hmac<Sha256>;

/// Encrypt data using AES-256-CBC with HMAC-SHA256 authentication.
///
/// Uses Encrypt-then-MAC: encrypts first, then computes HMAC over (IV || ciphertext).
pub fn encrypt(plaintext: &[u8], key: &StretchedKey) -> Result<EncryptedBlob, CryptoError> {
    // Generate random 128-bit IV
    let iv = random_bytes(16);

    // Encrypt with AES-256-CBC
    let ciphertext = Aes256CbcEnc::new((&key.enc_key).into(), iv.as_slice().into())
        .encrypt_padded_vec_mut::<Pkcs7>(plaintext);

    // Compute HMAC-SHA256 over (IV || ciphertext)
    let mut mac = HmacSha256::new_from_slice(&key.mac_key)
        .map_err(|e| CryptoError::Encryption(format!("HMAC init failed: {}", e)))?;
    mac.update(&iv);
    mac.update(&ciphertext);
    let mac_result = mac.finalize().into_bytes().to_vec();

    Ok(EncryptedBlob::new(ciphertext, iv, mac_result))
}

/// Decrypt data using AES-256-CBC with HMAC-SHA256 verification.
///
/// Verifies MAC first (constant-time), then decrypts.
pub fn decrypt(blob: &EncryptedBlob, key: &StretchedKey) -> Result<Vec<u8>, CryptoError> {
    // Verify IV length
    if blob.iv.len() != 16 {
        return Err(CryptoError::Encryption(format!(
            "Invalid IV length: expected 16, got {}",
            blob.iv.len()
        )));
    }

    // Verify MAC length
    if blob.mac.len() != 32 {
        return Err(CryptoError::Encryption(format!(
            "Invalid MAC length: expected 32, got {}",
            blob.mac.len()
        )));
    }

    // Compute expected MAC
    let mut mac = HmacSha256::new_from_slice(&key.mac_key)
        .map_err(|e| CryptoError::Encryption(format!("HMAC init failed: {}", e)))?;
    mac.update(&blob.iv);
    mac.update(&blob.ciphertext);
    let expected_mac = mac.finalize().into_bytes();

    // Constant-time MAC comparison to prevent timing attacks
    if expected_mac.ct_eq(&blob.mac).unwrap_u8() != 1 {
        return Err(CryptoError::MacVerification);
    }

    // Decrypt with AES-256-CBC
    let iv: [u8; 16] = blob.iv.as_slice().try_into().unwrap();
    let plaintext = Aes256CbcDec::new((&key.enc_key).into(), (&iv).into())
        .decrypt_padded_vec_mut::<Pkcs7>(&blob.ciphertext)
        .map_err(|e| CryptoError::Encryption(format!("Decryption failed: {}", e)))?;

    Ok(plaintext)
}

/// Encrypt a symmetric key for storage.
///
/// Wraps a SymmetricKey using the stretched key.
pub fn encrypt_key(key_to_encrypt: &SymmetricKey, wrap_key: &StretchedKey) -> Result<EncryptedBlob, CryptoError> {
    encrypt(key_to_encrypt.as_bytes(), wrap_key)
}

/// Decrypt a stored symmetric key.
///
/// Returns a SymmetricKey which automatically zeroizes on drop.
pub fn decrypt_key(blob: &EncryptedBlob, wrap_key: &StretchedKey) -> Result<SymmetricKey, CryptoError> {
    let mut decrypted = decrypt(blob, wrap_key)?;
    if decrypted.len() != 32 {
        decrypted.zeroize();
        return Err(CryptoError::InvalidKeyLength {
            expected: 32,
            got: decrypted.len(),
        });
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&decrypted);
    decrypted.zeroize();
    Ok(SymmetricKey(key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::StretchedKey;

    fn test_key() -> StretchedKey {
        StretchedKey {
            enc_key: [42u8; 32],
            mac_key: [43u8; 32],
        }
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = test_key();
        let plaintext = b"Hello, World! This is a test message.";

        let blob = encrypt(plaintext, &key).unwrap();
        let decrypted = decrypt(&blob, &key).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encrypt_produces_different_ciphertexts() {
        let key = test_key();
        let plaintext = b"Same message";

        let blob1 = encrypt(plaintext, &key).unwrap();
        let blob2 = encrypt(plaintext, &key).unwrap();

        // Same plaintext should produce different ciphertext (random IV)
        assert_ne!(blob1.ciphertext, blob2.ciphertext);
        assert_ne!(blob1.iv, blob2.iv);

        // But both should decrypt to the same plaintext
        assert_eq!(decrypt(&blob1, &key).unwrap(), plaintext);
        assert_eq!(decrypt(&blob2, &key).unwrap(), plaintext);
    }

    #[test]
    fn test_mac_verification_fails_on_tamper() {
        let key = test_key();
        let plaintext = b"Sensitive data";

        let mut blob = encrypt(plaintext, &key).unwrap();
        // Tamper with ciphertext
        blob.ciphertext[0] ^= 0xff;

        let result = decrypt(&blob, &key);
        assert!(matches!(result, Err(CryptoError::MacVerification)));
    }

    #[test]
    fn test_wrong_key_fails() {
        let key1 = test_key();
        let key2 = StretchedKey {
            enc_key: [44u8; 32],
            mac_key: [45u8; 32],
        };

        let plaintext = b"Secret message";
        let blob = encrypt(plaintext, &key1).unwrap();

        let result = decrypt(&blob, &key2);
        assert!(matches!(result, Err(CryptoError::MacVerification)));
    }

    #[test]
    fn test_encrypt_decrypt_empty() {
        let key = test_key();
        let plaintext = b"";

        let blob = encrypt(plaintext, &key).unwrap();
        let decrypted = decrypt(&blob, &key).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encrypt_decrypt_key() {
        let wrap_key = test_key();
        let key_to_encrypt = SymmetricKey([99u8; 32]);

        let blob = encrypt_key(&key_to_encrypt, &wrap_key).unwrap();
        let decrypted = decrypt_key(&blob, &wrap_key).unwrap();

        assert_eq!(decrypted.as_bytes(), key_to_encrypt.as_bytes());
    }

    #[test]
    fn test_invalid_iv_length() {
        let key = test_key();
        let blob = EncryptedBlob::new(vec![0u8; 32], vec![0u8; 8], vec![0u8; 32]);

        let result = decrypt(&blob, &key);
        assert!(matches!(result, Err(CryptoError::Encryption(_))));
    }

    #[test]
    fn test_invalid_mac_length() {
        let key = test_key();
        let blob = EncryptedBlob::new(vec![0u8; 32], vec![0u8; 16], vec![0u8; 16]);

        let result = decrypt(&blob, &key);
        assert!(matches!(result, Err(CryptoError::Encryption(_))));
    }
}
