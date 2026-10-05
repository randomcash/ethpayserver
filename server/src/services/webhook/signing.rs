//! HMAC-SHA256 webhook payload signing.

/// Sign a payload with HMAC-SHA256.
///
/// Returns a signature in the format `sha256=<hex>`.
pub fn sign_webhook_payload(payload: &str, secret: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    #[allow(
        clippy::expect_used,
        reason = "HmacSha256::new_from_slice is infallible for any key length"
    )]
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(payload.as_bytes());
    let result = mac.finalize();

    format!("sha256={}", hex::encode(result.into_bytes()))
}

/// Sign a payload together with the time this delivery attempt is sent.
///
/// Returns `t=<unix>,v1=<hex>` where `v1` is the HMAC-SHA256 of `"<unix>.<payload>"`.
/// Binding the send time into the MAC lets a receiver reject a captured request
/// once it is older than its tolerance; the body alone cannot give that, because
/// retries redeliver identical bytes for up to a day.
pub fn sign_webhook_payload_timestamped(payload: &str, secret: &str, sent_at_unix: i64) -> String {
    let signed = format!("{sent_at_unix}.{payload}");
    let sig = sign_webhook_payload(&signed, secret);
    let hex = sig.strip_prefix("sha256=").unwrap_or(&sig);
    format!("t={sent_at_unix},v1={hex}")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_sign_webhook_payload() {
        let payload = r#"{"test":"data"}"#;
        let secret = "my-secret-key";

        let signature = sign_webhook_payload(payload, secret);

        // Signature should start with "sha256="
        assert!(signature.starts_with("sha256="));

        // Signature should be deterministic
        let signature2 = sign_webhook_payload(payload, secret);
        assert_eq!(signature, signature2);

        // Different secret should produce different signature
        let signature3 = sign_webhook_payload(payload, "different-secret");
        assert_ne!(signature, signature3);

        // Different payload should produce different signature
        let signature4 = sign_webhook_payload(r#"{"test":"other"}"#, secret);
        assert_ne!(signature, signature4);
    }

    #[test]
    fn timestamped_signature_binds_time_body_and_secret() {
        let (body, secret) = (r#"{"a":1}"#, "k");
        let sig = sign_webhook_payload_timestamped(body, secret, 1_700_000_000);
        assert!(sig.starts_with("t=1700000000,v1="));
        // The MAC is over "t.body", so it equals the plain signature of that string.
        let expected = sign_webhook_payload(&format!("1700000000.{body}"), secret);
        assert_eq!(
            sig,
            format!("t=1700000000,v1={}", &expected["sha256=".len()..])
        );
        // A replay with a rewritten timestamp must not verify against the old MAC.
        let later = sign_webhook_payload_timestamped(body, secret, 1_700_000_999);
        assert_ne!(sig.split("v1=").nth(1), later.split("v1=").nth(1));
        assert_ne!(
            sig,
            sign_webhook_payload_timestamped(body, "other", 1_700_000_000)
        );
        assert_ne!(
            sig,
            sign_webhook_payload_timestamped("{}", secret, 1_700_000_000)
        );
    }
}
