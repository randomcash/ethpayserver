//! Stable idempotency keys for webhook events.
//!
//! Delivery is at-least-once (see the module docs), so a subscriber will
//! sometimes see the same logical event twice. The key it dedupes on has to be
//! a function of *what happened*, not of *when we sent it*: a fresh UUID or a
//! send-time timestamp changes on every emission and so dedupes nothing beyond
//! a single queued job's retries.

use sha2::{Digest, Sha256};

use super::WebhookEventType;

/// Build the stable idempotency key for a logical webhook event.
///
/// The key is `evt_` followed by the hex SHA-256 of the event's identity:
///
/// ```text
/// <event_type>\n<invoice_id>\n<transition>
/// ```
///
/// `transition` names the specific state change within that invoice — the
/// chain and transaction hash for a payment event, the reverted-to status and
/// the retracted transactions for a reorg. It is empty for events an invoice
/// can only ever undergo once (expiry, cancellation), where the invoice id and
/// the event type are already the whole identity.
///
/// The payload version is deliberately *not* an input: bumping the version
/// does not make it a different thing that happened, and mixing it in would
/// silently break dedupe across the deploy that bumped it.
pub fn idempotency_key(event_type: WebhookEventType, invoice_id: &str, transition: &str) -> String {
    let mut hasher = Sha256::new();
    // Newline-separated rather than concatenated: without a separator,
    // ("ab", "c") and ("a", "bc") hash identically.
    hasher.update(event_type.as_str().as_bytes());
    hasher.update(b"\n");
    hasher.update(invoice_id.as_bytes());
    hasher.update(b"\n");
    hasher.update(transition.as_bytes());
    format!("evt_{}", hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    /// The property the whole scheme rests on: build it twice, get one key.
    #[test]
    fn test_same_logical_event_produces_the_same_key() {
        let a = idempotency_key(
            WebhookEventType::PaymentConfirmed,
            "inv_1",
            "eip155:1:0xdeadbeef",
        );
        let b = idempotency_key(
            WebhookEventType::PaymentConfirmed,
            "inv_1",
            "eip155:1:0xdeadbeef",
        );
        assert_eq!(a, b);
    }

    /// And the other half: no two distinct events share a key. Each input
    /// varies alone, so a key that ignores one of them fails exactly here.
    #[test]
    fn test_different_events_do_not_collide() {
        let base = idempotency_key(
            WebhookEventType::PaymentConfirmed,
            "inv_1",
            "eip155:1:0xdeadbeef",
        );

        let other_type = idempotency_key(
            WebhookEventType::PaymentDetected,
            "inv_1",
            "eip155:1:0xdeadbeef",
        );
        let other_invoice = idempotency_key(
            WebhookEventType::PaymentConfirmed,
            "inv_2",
            "eip155:1:0xdeadbeef",
        );
        let other_tx = idempotency_key(
            WebhookEventType::PaymentConfirmed,
            "inv_1",
            "eip155:1:0xfeedface",
        );
        let other_chain = idempotency_key(
            WebhookEventType::PaymentConfirmed,
            "inv_1",
            "eip155:137:0xdeadbeef",
        );

        let keys = [base, other_type, other_invoice, other_tx, other_chain];
        for (i, a) in keys.iter().enumerate() {
            for b in &keys[i + 1..] {
                assert_ne!(a, b, "distinct events must not share an idempotency key");
            }
        }
    }

    /// Field boundaries are real: without the separator, moving a character
    /// across one would produce the same key.
    #[test]
    fn test_field_boundaries_are_not_ambiguous() {
        let a = idempotency_key(WebhookEventType::InvoiceExpired, "inv_1", "x");
        let b = idempotency_key(WebhookEventType::InvoiceExpired, "inv_1x", "");
        assert_ne!(a, b);
    }

    #[test]
    fn test_key_shape() {
        let key = idempotency_key(WebhookEventType::InvoiceExpired, "inv_1", "");
        assert!(key.starts_with("evt_"));
        // "evt_" + 64 hex chars of SHA-256.
        assert_eq!(key.len(), 68);
        assert!(key[4..].chars().all(|c| c.is_ascii_hexdigit()));
    }
}
