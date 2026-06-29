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
}
