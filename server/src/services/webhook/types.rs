//! Webhook event, payload, and payment-info types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Webhook event types that trigger notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookEventType {
    /// Payment detected (pending → processing)
    PaymentDetected,
    /// Payment confirmed (processing → paid)
    PaymentConfirmed,
    /// Invoice expired (pending → expired)
    InvoiceExpired,
    /// Invoice cancelled
    InvoiceCancelled,
    /// Late payment received (expired → late_paid)
    /// Requires merchant review before fulfillment.
    LatePaid,
    /// Refund transaction initiated.
    RefundInitiated,
    /// Refund transaction confirmed on-chain.
    RefundConfirmed,
    /// Refund transaction failed.
    RefundFailed,
    /// Payout/sweep transaction initiated.
    PayoutInitiated,
    /// Payout transaction confirmed on-chain.
    PayoutConfirmed,
    /// Payout transaction failed.
    PayoutFailed,
}

impl std::fmt::Display for WebhookEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PaymentDetected => write!(f, "payment_detected"),
            Self::PaymentConfirmed => write!(f, "payment_confirmed"),
            Self::InvoiceExpired => write!(f, "invoice_expired"),
            Self::InvoiceCancelled => write!(f, "invoice_cancelled"),
            Self::LatePaid => write!(f, "late_paid"),
            Self::RefundInitiated => write!(f, "refund_initiated"),
            Self::RefundConfirmed => write!(f, "refund_confirmed"),
            Self::RefundFailed => write!(f, "refund_failed"),
            Self::PayoutInitiated => write!(f, "payout_initiated"),
            Self::PayoutConfirmed => write!(f, "payout_confirmed"),
            Self::PayoutFailed => write!(f, "payout_failed"),
        }
    }
}

/// Webhook payload sent to merchant endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookPayload {
    /// Unique event ID for idempotency.
    pub event_id: Uuid,

    /// Event type.
    pub event_type: WebhookEventType,

    /// Timestamp when the event occurred.
    pub timestamp: DateTime<Utc>,

    /// Invoice ID.
    pub invoice_id: String,

    /// Store ID.
    pub store_id: Uuid,

    /// Current invoice status.
    pub status: String,

    /// Amount requested (in smallest unit).
    pub amount: String,

    /// Amount received so far (in smallest unit).
    pub amount_received: String,

    /// Asset symbol (e.g., "ETH", "USDT").
    pub asset_symbol: String,

    /// Chain ID (EIP-155).
    pub chain_id: u64,

    /// Network name (null for testnets/custom chains).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,

    /// Payment details (if applicable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment: Option<WebhookPaymentInfo>,
}

/// Payment information included in webhook payloads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookPaymentInfo {
    /// Transaction hash.
    pub tx_hash: String,

    /// Sender address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_address: Option<String>,

    /// Block number where payment was included.
    /// Confirmations can be computed as: current_block - block_number + 1
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_number: Option<u64>,

    /// Whether the payment has reached required confirmations.
    pub confirmed: bool,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_webhook_event_type_display() {
        assert_eq!(
            WebhookEventType::PaymentDetected.to_string(),
            "payment_detected"
        );
        assert_eq!(
            WebhookEventType::PaymentConfirmed.to_string(),
            "payment_confirmed"
        );
        assert_eq!(
            WebhookEventType::InvoiceExpired.to_string(),
            "invoice_expired"
        );
        assert_eq!(
            WebhookEventType::InvoiceCancelled.to_string(),
            "invoice_cancelled"
        );
        assert_eq!(WebhookEventType::LatePaid.to_string(), "late_paid");
        assert_eq!(
            WebhookEventType::RefundInitiated.to_string(),
            "refund_initiated"
        );
        assert_eq!(
            WebhookEventType::RefundConfirmed.to_string(),
            "refund_confirmed"
        );
        assert_eq!(WebhookEventType::RefundFailed.to_string(), "refund_failed");
        assert_eq!(
            WebhookEventType::PayoutInitiated.to_string(),
            "payout_initiated"
        );
        assert_eq!(
            WebhookEventType::PayoutConfirmed.to_string(),
            "payout_confirmed"
        );
        assert_eq!(WebhookEventType::PayoutFailed.to_string(), "payout_failed");
    }

    #[test]
    fn test_webhook_payload_serialization() {
        let payload = WebhookPayload {
            event_id: Uuid::new_v4(),
            event_type: WebhookEventType::PaymentConfirmed,
            timestamp: Utc::now(),
            invoice_id: "inv_123".to_string(),
            store_id: Uuid::new_v4(),
            status: "paid".to_string(),
            amount: "1000000000000000000".to_string(),
            amount_received: "1000000000000000000".to_string(),
            asset_symbol: "ETH".to_string(),
            chain_id: 1,
            network: Some("ethereum".to_string()),
            payment: Some(WebhookPaymentInfo {
                tx_hash: "0x1234".to_string(),
                from_address: Some("0xabcd".to_string()),
                block_number: Some(12345),
                confirmed: true,
            }),
        };

        // Should serialize to JSON
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("payment_confirmed"));
        assert!(json.contains("inv_123"));

        // Should deserialize back
        let deserialized: WebhookPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.invoice_id, payload.invoice_id);
        assert!(deserialized.payment.is_some());
    }
}
