//! Webhook error types.

/// Error type for webhook operations.
#[derive(Debug, thiserror::Error)]
pub enum WebhookError {
    #[error("redis error: {0}")]
    Redis(String),

    #[error("http error: {0}")]
    Http(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("database error: {0}")]
    Database(#[from] types::RepositoryError),
}
