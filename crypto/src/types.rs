//! Core types for the crypto module.

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Key Derivation Function parameters for Argon2id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdfParams {
    /// Algorithm identifier (always "argon2id").
    pub algorithm: String,
    /// Memory cost in KB.
    pub memory_kb: u32,
    /// Number of iterations.
    pub iterations: u32,
    /// Degree of parallelism.
    pub parallelism: u32,
    /// Random salt (16 bytes).
    #[serde(with = "base64_bytes")]
    pub salt: Vec<u8>,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            algorithm: "argon2id".to_string(),
            memory_kb: 65536, // 64 MB
            iterations: 3,
            parallelism: 4,
            salt: vec![0u8; 16], // Will be randomized
        }
    }
}

impl KdfParams {
    /// Create new KDF params with a random salt.
    pub fn new_random() -> Self {
        use rand::RngCore;
        let mut salt = vec![0u8; 16];
        rand::thread_rng().fill_bytes(&mut salt);
        Self {
            salt,
            ..Default::default()
        }
    }
}

/// Encrypted data blob with authentication (Encrypt-then-MAC).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedBlob {
    /// AES-256-CBC ciphertext.
    #[serde(with = "base64_bytes")]
    pub ciphertext: Vec<u8>,
    /// Random 128-bit initialization vector.
    #[serde(with = "base64_bytes")]
    pub iv: Vec<u8>,
    /// HMAC-SHA256 of (iv || ciphertext).
    #[serde(with = "base64_bytes")]
    pub mac: Vec<u8>,
}

impl EncryptedBlob {
    /// Create a new encrypted blob.
    pub fn new(ciphertext: Vec<u8>, iv: Vec<u8>, mac: Vec<u8>) -> Self {
        Self {
            ciphertext,
            iv,
            mac,
        }
    }
}

/// A 256-bit symmetric key that zeroizes on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SymmetricKey(pub [u8; 32]);

impl SymmetricKey {
    pub fn from_slice(slice: &[u8]) -> Option<Self> {
        if slice.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(slice);
            Some(Self(key))
        } else {
            None
        }
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl AsRef<[u8]> for SymmetricKey {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// A stretched key containing both encryption and MAC keys.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct StretchedKey {
    /// 256-bit encryption key.
    pub enc_key: [u8; 32],
    /// 256-bit MAC key.
    pub mac_key: [u8; 32],
}

impl StretchedKey {
    pub fn from_bytes(bytes: &[u8; 64]) -> Self {
        let mut enc_key = [0u8; 32];
        let mut mac_key = [0u8; 32];
        enc_key.copy_from_slice(&bytes[..32]);
        mac_key.copy_from_slice(&bytes[32..]);
        Self { enc_key, mac_key }
    }
}

/// Master password hash for server-side verification.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct MasterPasswordHash(pub [u8; 32]);

impl MasterPasswordHash {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Encode as base64 for transmission.
    pub fn to_base64(&self) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&self.0)
    }
}

/// Serde helper for base64-encoded bytes.
mod base64_bytes {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        serializer.serialize_str(&encoded)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(&s)
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kdf_params_default() {
        let params = KdfParams::default();
        assert_eq!(params.algorithm, "argon2id");
        assert_eq!(params.memory_kb, 65536);
        assert_eq!(params.iterations, 3);
        assert_eq!(params.parallelism, 4);
    }

    #[test]
    fn test_kdf_params_random_salt() {
        let params1 = KdfParams::new_random();
        let params2 = KdfParams::new_random();
        assert_ne!(params1.salt, params2.salt);
        assert_eq!(params1.salt.len(), 16);
    }

    #[test]
    fn test_symmetric_key_from_slice() {
        let bytes = [42u8; 32];
        let key = SymmetricKey::from_slice(&bytes).unwrap();
        assert_eq!(key.as_bytes(), &bytes);

        // Wrong size should fail
        assert!(SymmetricKey::from_slice(&[0u8; 16]).is_none());
    }

    #[test]
    fn test_encrypted_blob_serialization() {
        let blob = EncryptedBlob::new(vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]);

        let json = serde_json::to_string(&blob).unwrap();
        let parsed: EncryptedBlob = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.ciphertext, blob.ciphertext);
        assert_eq!(parsed.iv, blob.iv);
        assert_eq!(parsed.mac, blob.mac);
    }
}
