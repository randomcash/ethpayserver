//! Error types for the auth module.

use thiserror::Error;

/// Errors that can occur during authentication operations.
#[derive(Debug, Error)]
pub enum AuthError {
    /// User already exists.
    #[error("User already exists: {0}")]
    UserExists(String),

    /// User not found.
    #[error("User not found: {0}")]
    UserNotFound(String),

    /// Invalid credentials (user not found or passkey verification failed).
    /// Used as a generic error to prevent user enumeration.
    #[error("Invalid credentials")]
    InvalidCredentials,

    /// Device not found.
    #[error("Device not found: {0}")]
    DeviceNotFound(String),

    /// Device already exists.
    /// Reserved for future use when implementing device duplicate detection.
    #[allow(dead_code)]
    #[error("Device already exists: {0}")]
    DeviceExists(String),

    /// Passkey not found.
    #[error("Passkey not found: {0}")]
    PasskeyNotFound(String),

    /// Passkey verification failed.
    #[error("Passkey verification failed")]
    PasskeyVerificationFailed,

    /// Passkey challenge expired or not found.
    #[error("Passkey challenge expired or not found")]
    PasskeyChallengeExpired,

    /// WebAuthn error.
    #[error("WebAuthn error: {0}")]
    WebAuthn(String),

    /// Cannot revoke current device.
    #[error("Cannot revoke the device you are currently using")]
    CannotRevokeCurrentDevice,

    /// Session expired or invalid.
    #[error("Session expired or invalid")]
    SessionInvalid,

    /// Invalid recovery mnemonic.
    #[error("Invalid recovery mnemonic")]
    InvalidRecoveryMnemonic,

    /// Recovery not available (mnemonic not set up).
    /// Note: Not currently returned to prevent user enumeration.
    /// See InvalidRecoveryMnemonic instead.
    #[allow(dead_code)]
    #[error("Recovery not available for this account")]
    RecoveryNotAvailable,

    /// Invalid recovery setup.
    /// Reserved for future use. Recovery is now always required during registration.
    #[allow(dead_code)]
    #[error("Invalid recovery setup")]
    InvalidRecoverySetup,

    /// Invalid email format.
    #[error("Invalid email format: {0}")]
    InvalidEmail(String),

    /// Cryptographic operation failed.
    #[error("Crypto error: {0}")]
    Crypto(#[from] crypto::CryptoError),

    /// Repository/database error.
    #[error("Repository error: {0}")]
    Repository(String),

    /// Rate limit exceeded.
    /// Reserved for future use when implementing IP-based rate limiting.
    #[allow(dead_code)]
    #[error("Rate limit exceeded, try again later")]
    RateLimited,

    /// Account locked (too many failed attempts).
    #[error("Account locked due to too many failed attempts")]
    AccountLocked,

    /// Maximum devices reached.
    #[error("Maximum devices reached ({0}). Please remove a device first.")]
    MaxDevicesReached(u32),

    /// Maximum passkeys reached.
    #[error("Maximum passkeys reached ({0}). Please remove a passkey first.")]
    MaxPasskeysReached(u32),
}

/// Result type for auth operations.
pub type Result<T> = std::result::Result<T, AuthError>;
