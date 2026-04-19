//! Error types for the EVM crate.

use thiserror::Error;

/// Errors that can occur in EVM operations.
#[derive(Debug, Clone, Error)]
pub enum EvmError {
    /// RPC provider error.
    #[error("RPC error: {0}")]
    Rpc(String),

    /// Connection error.
    #[error("connection error: {0}")]
    Connection(String),

    /// Invalid chain configuration.
    #[error("invalid chain config: {0}")]
    InvalidChainConfig(String),

    /// Unsupported network.
    #[error("unsupported network: {0}")]
    UnsupportedNetwork(String),

    /// HD wallet derivation error.
    #[error("wallet derivation error: {0}")]
    WalletDerivation(String),

    /// Invalid mnemonic.
    #[error("invalid mnemonic: {0}")]
    InvalidMnemonic(String),

    /// Invalid derivation path.
    #[error("invalid derivation path: {0}")]
    InvalidDerivationPath(String),

    /// Invalid extended public key.
    #[error("invalid xpub: {0}")]
    InvalidXpub(String),

    /// Address parsing error.
    #[error("invalid address: {0}")]
    InvalidAddress(String),

    /// Contract interaction error.
    #[error("contract error: {0}")]
    Contract(String),

    /// Token not found in registry.
    #[error("token not found: {0}")]
    TokenNotFound(String),

    /// Transaction not found.
    #[error("transaction not found: {0}")]
    TransactionNotFound(String),

    /// Timeout waiting for operation.
    #[error("timeout: {0}")]
    Timeout(String),

    /// Configuration error.
    #[error("config error: {0}")]
    Config(String),

    /// Subscription error.
    #[error("subscription error: {0}")]
    Subscription(String),

    /// Monitor error.
    #[error("monitor error: {0}")]
    Monitor(String),

    /// Generic internal error.
    #[error("internal error: {0}")]
    Internal(String),
}

/// Result type for EVM operations.
pub type EvmResult<T> = Result<T, EvmError>;
