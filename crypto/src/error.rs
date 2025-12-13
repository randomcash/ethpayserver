//! Error types for the crypto module.

use thiserror::Error;

/// Errors that can occur during cryptographic operations.
#[derive(Debug, Error)]
pub enum CryptoError {
    /// Key derivation function error.
    #[error("KDF error: {0}")]
    Kdf(String),

    /// Encryption/decryption error.
    #[error("Encryption error: {0}")]
    Encryption(String),

    /// MAC verification failed.
    #[error("MAC verification failed")]
    MacVerification,

    /// Invalid key length.
    #[error("Invalid key length: expected {expected}, got {got}")]
    InvalidKeyLength { expected: usize, got: usize },

    /// Invalid signature.
    #[error("Invalid signature")]
    InvalidSignature,

    /// Mnemonic error.
    #[error("Mnemonic error: {0}")]
    Mnemonic(String),

    /// Base64 decoding error.
    #[error("Base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),
}
