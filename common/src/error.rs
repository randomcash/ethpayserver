//! Error types for the PayServer ecosystem.

use thiserror::Error;

use crate::types::{Currency, InvoiceId, InvoiceStatus, PaymentMethod};

/// Main error type for PayServer operations.
#[derive(Debug, Error)]
pub enum PayServerError {
    /// Invoice not found.
    #[error("invoice not found: {0}")]
    InvoiceNotFound(InvoiceId),

    /// Invoice has expired.
    #[error("invoice expired: {0}")]
    InvoiceExpired(InvoiceId),

    /// Invoice is in an invalid state for the requested operation.
    #[error("invalid invoice state: invoice {invoice_id} is {current_status}, expected one of {expected:?}")]
    InvalidInvoiceState {
        invoice_id: InvoiceId,
        current_status: InvoiceStatus,
        expected: Vec<InvoiceStatus>,
    },

    /// Payment method not supported.
    #[error("unsupported payment method: {0:?}")]
    UnsupportedPaymentMethod(PaymentMethod),

    /// Currency not supported.
    #[error("unsupported currency: {0}")]
    UnsupportedCurrency(Currency),

    /// Invalid amount.
    #[error("invalid amount: {0}")]
    InvalidAmount(String),

    /// Database error.
    #[error("database error: {0}")]
    Database(String),

    /// Network/RPC error.
    #[error("network error: {0}")]
    Network(String),

    /// Blockchain node error.
    #[error("blockchain error: {0}")]
    Blockchain(String),

    /// Configuration error.
    #[error("configuration error: {0}")]
    Configuration(String),

    /// Webhook delivery failed.
    #[error("webhook delivery failed: {0}")]
    WebhookFailed(String),

    /// Rate limit exceeded.
    #[error("rate limit exceeded")]
    RateLimitExceeded,

    /// Internal error.
    #[error("internal error: {0}")]
    Internal(String),

    /// Serialization/deserialization error.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Validation error.
    #[error("validation error: {0}")]
    Validation(String),
}

impl PayServerError {
    /// Returns true if this error is retryable.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            PayServerError::Network(_)
                | PayServerError::Blockchain(_)
                | PayServerError::WebhookFailed(_)
                | PayServerError::RateLimitExceeded
        )
    }

    /// Returns an HTTP status code appropriate for this error.
    pub fn http_status_code(&self) -> u16 {
        match self {
            PayServerError::InvoiceNotFound(_) => 404,
            PayServerError::InvoiceExpired(_) => 410,
            PayServerError::InvalidInvoiceState { .. } => 409,
            PayServerError::UnsupportedPaymentMethod(_) => 400,
            PayServerError::UnsupportedCurrency(_) => 400,
            PayServerError::InvalidAmount(_) => 400,
            PayServerError::Validation(_) => 400,
            PayServerError::RateLimitExceeded => 429,
            PayServerError::Configuration(_) => 500,
            PayServerError::Database(_) => 500,
            PayServerError::Network(_) => 502,
            PayServerError::Blockchain(_) => 502,
            PayServerError::WebhookFailed(_) => 502,
            PayServerError::Internal(_) => 500,
            PayServerError::Serialization(_) => 500,
        }
    }
}

/// Result type alias for PayServer operations.
pub type PayServerResult<T> = Result<T, PayServerError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_retryable() {
        assert!(PayServerError::Network("timeout".into()).is_retryable());
        assert!(PayServerError::RateLimitExceeded.is_retryable());
        assert!(!PayServerError::InvoiceNotFound(InvoiceId::new()).is_retryable());
    }

    #[test]
    fn test_error_http_status() {
        assert_eq!(
            PayServerError::InvoiceNotFound(InvoiceId::new()).http_status_code(),
            404
        );
        assert_eq!(PayServerError::RateLimitExceeded.http_status_code(), 429);
        assert_eq!(
            PayServerError::Validation("bad input".into()).http_status_code(),
            400
        );
    }
}
