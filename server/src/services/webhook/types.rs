//! Webhook event, payload, and payment-info types.
//!
//! See the module docs in `mod.rs` for the delivery guarantee, the
//! idempotency rule, and the payload compatibility rule. This file is the
//! vocabulary itself: every event a subscriber can ever receive is a variant
//! of [`WebhookEventType`], and every variant here is emitted by some code
//! path in this repository.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use types::{InvoiceData, PaymentData};
use uuid::Uuid;

use super::idempotency::idempotency_key;

/// Current webhook payload version, sent as `version` on every payload.
///
/// See "Payload compatibility" in the module docs: this is bumped only for a
/// change a subscriber cannot ignore, and additive fields never bump it.
pub const WEBHOOK_PAYLOAD_VERSION: u32 = 1;

/// Webhook event types that trigger notifications.
///
/// Every variant is emitted somewhere. A variant that no code path can produce
/// is a promise to subscribers that is silently never kept, so it does not
/// belong here — `refund_*` and `payout_*` were removed for exactly that
/// reason, since nothing in this server signs or broadcasts a refund or payout
/// transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookEventType {
    /// Payment detected (pending → processing)
    PaymentDetected,
    /// Payment confirmed (processing → paid)
    PaymentConfirmed,
    /// One or more previously reported payments were retracted by a chain
    /// reorganization, and the invoice status was reverted.
    ///
    /// This is a *retraction*, not a restatement: a subscriber that acted on
    /// an earlier `payment_detected` or `payment_confirmed` for the listed
    /// transactions must undo that action. `status` carries the status the
    /// invoice reverted to and `retracted_payments` the transactions that no
    /// longer exist on the canonical chain.
    PaymentReorged,
    /// Invoice expired (pending → expired)
    InvoiceExpired,
    /// Invoice cancelled
    InvoiceCancelled,
    /// Late payment received (expired → late_paid)
    /// Requires merchant review before fulfillment.
    LatePaid,
}

impl WebhookEventType {
    /// The wire name, as sent in `event_type` and the `X-Webhook-Event` header.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PaymentDetected => "payment_detected",
            Self::PaymentConfirmed => "payment_confirmed",
            Self::PaymentReorged => "payment_reorged",
            Self::InvoiceExpired => "invoice_expired",
            Self::InvoiceCancelled => "invoice_cancelled",
            Self::LatePaid => "late_paid",
        }
    }
}

impl std::fmt::Display for WebhookEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Webhook payload sent to merchant endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookPayload {
    /// Payload schema version. See [`WEBHOOK_PAYLOAD_VERSION`].
    ///
    /// Defaults to `0` when absent, which means "queued before versioning
    /// existed" — jobs already in the Redis queue across the deploy that
    /// introduced this field deserialize rather than being dropped.
    #[serde(default)]
    pub version: u32,

    /// Identifier for this delivery of this event.
    ///
    /// Constant across retries of one queued job, because the payload is built
    /// once and redelivered verbatim. It is *not* constant across two
    /// emissions of the same logical event (a handler re-run after a crash),
    /// so it is the wrong thing to dedupe on — use `idempotency_key`.
    pub event_id: Uuid,

    /// Stable idempotency key for the logical event.
    ///
    /// Derived from the event's identity — type, invoice, and the transition
    /// it reports — never from a random id or a send-time clock. The same
    /// logical event produces the same key every time it is built, so a
    /// subscriber can record keys it has processed and drop repeats. Empty
    /// only on payloads queued before this field existed.
    #[serde(default)]
    pub idempotency_key: String,

    /// Event type.
    pub event_type: WebhookEventType,

    /// Timestamp when the event occurred.
    pub timestamp: DateTime<Utc>,

    /// Invoice ID.
    pub invoice_id: String,

    /// Store ID.
    pub store_id: Uuid,

    /// Current invoice status.
    ///
    /// For `payment_reorged` this is the status the invoice was reverted *to*.
    pub status: String,

    /// Amount requested (in smallest unit).
    pub amount: String,

    /// Amount received so far (in smallest unit).
    pub amount_received: String,

    /// Asset symbol (e.g., "ETH", "USDT").
    pub asset_symbol: String,

    /// CAIP-2 chain identifier, e.g. `eip155:1`.
    ///
    /// Was a JSON number (the EIP-155 id) before CAIP-2. Absent entirely on
    /// invoice-level events, which involve no chain - it previously sent `0`
    /// there, which is not a chain, and briefly sent `""`, which is not an
    /// identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<String>,

    /// Human-readable chain name.
    ///
    /// Always absent since chain ids became CAIP-2. It used to be a name from
    /// a closed enum
    /// (`"ethereum"`), and that enum is gone: a CAIP-2 reference is mostly an
    /// opaque genesis hash, so no function can derive a name from one. The
    /// mapping lives in `chain_configs`; until this reads it, sending anything
    /// here would either be a guess or a duplicate of `chain_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,

    /// Payment details (if applicable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment: Option<WebhookPaymentInfo>,

    /// Payments retracted by this event.
    ///
    /// Present only on `payment_reorged`, where it lists every payment this
    /// reorg invalidated. A subscriber that credited any of these transactions
    /// must reverse that credit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retracted_payments: Option<Vec<WebhookPaymentInfo>>,
}

impl WebhookPayload {
    /// Build an invoice-level event: expiration or cancellation.
    ///
    /// No chain is involved, so `chain_id` and `network` are absent rather
    /// than carrying a placeholder.
    pub fn invoice_event(event_type: WebhookEventType, invoice: &InvoiceData) -> Self {
        Self::build(
            event_type,
            invoice,
            // An invoice expires once and is cancelled once, so the invoice id
            // and the event type already identify the transition.
            "",
            invoice.currency.clone(),
            None,
            None,
            None,
        )
    }

    /// Build a payment-level event: detected, confirmed, or late-paid.
    ///
    /// The chain and asset come from the payment; a network-agnostic invoice
    /// has neither until one is made.
    pub fn payment_event(
        event_type: WebhookEventType,
        invoice: &InvoiceData,
        payment: &PaymentData,
    ) -> Self {
        Self::build(
            event_type,
            invoice,
            // A transaction can pay an invoice once, so (event type, invoice,
            // chain, tx) is the identity of the transition it caused.
            &format!("{}:{}", payment.chain_id.as_str(), payment.tx_hash),
            payment.asset_symbol.clone(),
            Some(payment.chain_id.as_str().to_string()),
            Some(WebhookPaymentInfo::from_payment(payment)),
            None,
        )
    }

    /// Build the retraction event for a chain reorganization.
    ///
    /// `invoice` must be read *after* the status revert, so `status` and
    /// `amount_received` describe the world the subscriber is being moved to.
    /// `retracted` is every payment this reorg invalidated.
    pub fn payment_reorged(invoice: &InvoiceData, retracted: &[PaymentData]) -> Self {
        let chain_id = retracted.first().map(|p| p.chain_id.as_str().to_string());
        let asset_symbol = retracted
            .first()
            .map_or_else(|| invoice.currency.clone(), |p| p.asset_symbol.clone());

        // Sorted so that the key does not depend on row order coming back from
        // the store: the same reorg retracting the same transactions is the
        // same logical event however the rows are ordered.
        let mut tx_hashes: Vec<&str> = retracted.iter().map(|p| p.tx_hash.as_str()).collect();
        tx_hashes.sort_unstable();

        Self::build(
            WebhookEventType::PaymentReorged,
            invoice,
            &format!("{}:{}", invoice.status, tx_hashes.join(",")),
            asset_symbol,
            chain_id,
            None,
            Some(
                retracted
                    .iter()
                    .map(WebhookPaymentInfo::from_payment)
                    .collect(),
            ),
        )
    }

    #[allow(clippy::too_many_arguments)] // private constructor; every field is part of the wire contract
    fn build(
        event_type: WebhookEventType,
        invoice: &InvoiceData,
        transition: &str,
        asset_symbol: String,
        chain_id: Option<String>,
        payment: Option<WebhookPaymentInfo>,
        retracted_payments: Option<Vec<WebhookPaymentInfo>>,
    ) -> Self {
        let invoice_id = invoice.id.as_str().to_string();
        Self {
            version: WEBHOOK_PAYLOAD_VERSION,
            event_id: Uuid::new_v4(),
            idempotency_key: idempotency_key(event_type, &invoice_id, transition),
            event_type,
            timestamp: Utc::now(),
            invoice_id,
            store_id: invoice.store_id.0,
            status: invoice.status.to_string(),
            amount: invoice.amount.clone(),
            amount_received: invoice.amount_received.clone(),
            asset_symbol,
            chain_id,
            // See `network`: no name is derivable from a CAIP-2 identifier,
            // and duplicating chain_id here helps nobody.
            network: None,
            payment,
            retracted_payments,
        }
    }
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

impl WebhookPaymentInfo {
    /// Project a stored payment onto the wire shape.
    pub fn from_payment(payment: &PaymentData) -> Self {
        Self {
            tx_hash: payment.tx_hash.clone(),
            from_address: payment.from_address.clone(),
            block_number: payment.block_number,
            confirmed: payment.confirmed_at.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use types::{ChainId, InvoiceId, InvoiceStatus, StoreId};

    fn test_invoice(status: InvoiceStatus) -> InvoiceData {
        InvoiceData {
            id: InvoiceId::from_string("inv_123".to_string()),
            store_id: StoreId::new(),
            currency: "ETH".to_string(),
            status,
            amount: "1000".to_string(),
            amount_received: "0".to_string(),
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            metadata: None,
            customer_email: None,
            extra: None,
        }
    }

    fn test_payment(invoice: &InvoiceData, tx_hash: &str, block: u64) -> PaymentData {
        PaymentData {
            id: Uuid::new_v4(),
            invoice_id: invoice.id.clone(),
            payment_option_id: None,
            chain_id: ChainId::evm(1),
            asset_type: types::AssetType::Native,
            amount: "1000".to_string(),
            asset_symbol: "ETH".to_string(),
            token_address: None,
            tx_hash: tx_hash.to_string(),
            block_number: Some(block),
            detected_at: Utc::now(),
            confirmed_at: None,
            from_address: Some("0xabcd".to_string()),
            reorged: false,
            extra: None,
            credited_amount: None,
            rate_used: None,
            rate_applied_at: None,
        }
    }

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
            WebhookEventType::PaymentReorged.to_string(),
            "payment_reorged"
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
    }

    /// The `Display` name and the serde name are one vocabulary, not two. They
    /// drifted apart silently before, because nothing compared them.
    #[test]
    fn test_display_matches_serde_name() {
        for event in [
            WebhookEventType::PaymentDetected,
            WebhookEventType::PaymentConfirmed,
            WebhookEventType::PaymentReorged,
            WebhookEventType::InvoiceExpired,
            WebhookEventType::InvoiceCancelled,
            WebhookEventType::LatePaid,
        ] {
            let json = serde_json::to_string(&event).unwrap();
            assert_eq!(json, format!("\"{event}\""));
        }
    }

    #[test]
    fn test_webhook_payload_serialization() {
        let invoice = test_invoice(InvoiceStatus::Paid);
        let payment = test_payment(&invoice, "0x1234", 12345);
        let payload =
            WebhookPayload::payment_event(WebhookEventType::PaymentConfirmed, &invoice, &payment);

        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("payment_confirmed"));
        assert!(json.contains("inv_123"));
        assert!(json.contains("\"version\":1"));
        assert!(json.contains(&payload.idempotency_key));

        let deserialized: WebhookPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.invoice_id, payload.invoice_id);
        assert_eq!(deserialized.idempotency_key, payload.idempotency_key);
        assert_eq!(deserialized.version, WEBHOOK_PAYLOAD_VERSION);
        assert!(deserialized.payment.is_some());
        assert!(deserialized.retracted_payments.is_none());
    }

    /// A payload queued before `version`/`idempotency_key` existed is still in
    /// the Redis queue across the deploy that adds them. It must deserialize,
    /// not poison the delivery loop.
    #[test]
    fn test_pre_versioning_payload_still_deserializes() {
        let json = r#"{
            "event_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
            "event_type": "payment_detected",
            "timestamp": "2026-04-01T12:15:00Z",
            "invoice_id": "inv_old",
            "store_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
            "status": "processing",
            "amount": "1000",
            "amount_received": "1000",
            "asset_symbol": "ETH"
        }"#;

        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.version, 0);
        assert!(payload.idempotency_key.is_empty());
        assert!(payload.payment.is_none());
    }

    /// The retraction event has to answer three questions: which invoice, what
    /// is it now, and which transactions do I forget.
    #[test]
    fn test_payment_reorged_payload_carries_retraction() {
        let mut invoice = test_invoice(InvoiceStatus::Pending);
        invoice.amount_received = "0".to_string();
        let p1 = test_payment(&invoice, "0xaaa", 100);
        let p2 = test_payment(&invoice, "0xbbb", 101);

        let payload = WebhookPayload::payment_reorged(&invoice, &[p1, p2]);

        assert_eq!(payload.event_type, WebhookEventType::PaymentReorged);
        assert_eq!(payload.status, "pending");
        assert_eq!(payload.chain_id.as_deref(), Some("eip155:1"));
        assert!(payload.payment.is_none());

        let retracted = payload.retracted_payments.unwrap();
        let hashes: Vec<&str> = retracted.iter().map(|p| p.tx_hash.as_str()).collect();
        assert_eq!(hashes, vec!["0xaaa", "0xbbb"]);
        assert!(retracted.iter().all(|p| !p.confirmed));
    }

    /// A retraction must not be mistakable for a restatement of the original.
    #[test]
    fn test_retraction_key_differs_from_the_event_it_retracts() {
        let invoice = test_invoice(InvoiceStatus::Processing);
        let payment = test_payment(&invoice, "0xaaa", 100);

        let detected =
            WebhookPayload::payment_event(WebhookEventType::PaymentDetected, &invoice, &payment);
        let reorged = WebhookPayload::payment_reorged(&invoice, std::slice::from_ref(&payment));

        assert_ne!(detected.idempotency_key, reorged.idempotency_key);
        assert_ne!(detected.event_type, reorged.event_type);
    }
}
