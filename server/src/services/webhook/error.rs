//! Webhook error types.

/// Error type for webhook operations.
#[derive(Debug, thiserror::Error)]
pub enum WebhookError {
    #[error("redis error: {0}")]
    Redis(String),

    #[error("http error: {0}")]
    Http(String),

    /// The request never reached the merchant's endpoint at all - DNS
    /// failed, the connection was refused, or nothing answered before the
    /// configured timeout. Distinct from `Http`, which covers a request
    /// that *did* reach the endpoint (a non-success status) or failed to
    /// build: those can reflect a fault in our own signing or request
    /// construction, so only this variant is a fact about the merchant's
    /// server rather than ours.
    #[error("webhook endpoint unreachable: {0}")]
    Unreachable(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("database error: {0}")]
    Database(#[from] types::RepositoryError),
}
