//! Cryptographic primitives for PayServer.
//!
//! Implements Bitwarden-style client-side encryption:
//!
//! ## Key Hierarchy
//!
//! ```text
//! Password + Email
//!        |
//!        v (Argon2id)
//!   Master Key (256-bit)
//!        |
//!   +----+----+
//!   |         | (HKDF)
//!   v         v
//! enc_key   mac_key   <- StretchedKey (512-bit)
//!   |
//!   v (encrypt)
//! [Encrypted Symmetric Key] <- stored on server
//!   |
//!   v (decrypt with stretched key)
//! Symmetric Key (256-bit, random)
//!   |
//!   v (AES-256-CBC + HMAC-SHA256)
//! [Encrypted User Data]
//! ```
//!
//! ## Modules
//!
//! - [`kdf`]: Key derivation (Argon2id, HKDF-SHA256)
//! - [`symmetric`]: Authenticated encryption (AES-256-CBC + HMAC-SHA256)
//! - [`asymmetric`]: Key exchange and signing (X25519, Ed25519)
//! - [`mnemonic`]: BIP39 recovery phrases
//! - [`types`]: Core cryptographic types
//! - [`error`]: Error types
//!
//! ## Example
//!
//! ```rust
//! use crypto::{kdf, symmetric, types::KdfParams};
//!
//! // Derive master key from password
//! let params = KdfParams::new_random();
//! let master_key = kdf::derive_master_key(b"password", "user@example.com", &params).unwrap();
//!
//! // Stretch to get encryption and MAC keys
//! let stretched = kdf::stretch_master_key(&master_key).unwrap();
//!
//! // Encrypt some data
//! let plaintext = b"Secret data";
//! let blob = symmetric::encrypt(plaintext, &stretched).unwrap();
//!
//! // Decrypt it back
//! let decrypted = symmetric::decrypt(&blob, &stretched).unwrap();
//! assert_eq!(decrypted, plaintext);
//! ```

pub mod asymmetric;
pub mod error;
pub mod kdf;
pub mod mnemonic;
pub mod symmetric;
pub mod types;

// Re-export commonly used items
pub use error::CryptoError;
pub use types::{EncryptedBlob, KdfParams, MasterPasswordHash, StretchedKey, SymmetricKey};

// Re-export key operations
pub use kdf::{derive_master_key, derive_master_password_hash, stretch_master_key};
pub use symmetric::{decrypt, encrypt};

// Re-export asymmetric types
pub use asymmetric::{Ed25519KeyPair, X25519KeyPair};

// Re-export mnemonic
pub use mnemonic::RecoveryMnemonic;
