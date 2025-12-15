//! Error types for data service operations.

use thiserror::Error;

/// Error types for repository operations.
#[derive(Debug, Error)]
pub enum RepositoryError {
    /// Database error.
    #[error("database error: {0}")]
    Database(String),

    /// Entity not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// Serialization/deserialization error.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Invalid data (e.g., unsupported enum variant, data corruption).
    #[error("invalid data: {0}")]
    InvalidData(String),
}

/// Result type for repository operations.
pub type RepositoryResult<T> = Result<T, RepositoryError>;
