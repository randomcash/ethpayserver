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
    /// construction, so this variant is at least never that.
    ///
    /// It is not, on its own, proof the fault is theirs rather than ours:
    /// the same DNS/refused/timeout shape is what payserver's own egress
    /// breaking looks like too, since nothing here can reach anyone either
    /// way. See `permanent_failure_is_merchant_unreachable` in
    /// `server::services::webhook::service` for the isolation check that
    /// tells the two apart before this variant is allowed to demote a log.
    #[error("webhook endpoint unreachable: {0}")]
    Unreachable(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("database error: {0}")]
    Database(#[from] types::RepositoryError),
}
