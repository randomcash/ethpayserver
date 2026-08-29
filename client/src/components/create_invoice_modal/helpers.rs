//! Pure form helpers for the create-invoice modal.

/// Build metadata JSON from optional order_id and buyer_email fields.
pub(super) fn build_metadata(order_id: &str, buyer_email: &str) -> Option<serde_json::Value> {
    if order_id.is_empty() && buyer_email.is_empty() {
        return None;
    }
    let mut map = serde_json::Map::new();
    if !order_id.is_empty() {
        map.insert(
            "order_id".to_string(),
            serde_json::Value::String(order_id.to_string()),
        );
    }
    if !buyer_email.is_empty() {
        map.insert(
            "buyer_email".to_string(),
            serde_json::Value::String(buyer_email.to_string()),
        );
    }
    Some(serde_json::Value::Object(map))
}

/// Basic URL validation: must start with http:// or https:// followed by content.
pub(super) fn is_valid_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("http://") && url.len() > "http://".len())
        || (lower.starts_with("https://") && url.len() > "https://".len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_metadata_both_fields() {
        let meta = build_metadata("ORD-123", "buyer@example.com");
        let meta = meta.expect("should be Some");
        assert_eq!(meta["order_id"], "ORD-123");
        assert_eq!(meta["buyer_email"], "buyer@example.com");
    }

    #[test]
    fn test_build_metadata_order_id_only() {
        let meta = build_metadata("ORD-456", "");
        let meta = meta.expect("should be Some");
        assert_eq!(meta["order_id"], "ORD-456");
        assert!(meta.get("buyer_email").is_none());
    }

    #[test]
    fn test_build_metadata_email_only() {
        let meta = build_metadata("", "test@example.com");
        let meta = meta.expect("should be Some");
        assert!(meta.get("order_id").is_none());
        assert_eq!(meta["buyer_email"], "test@example.com");
    }

    #[test]
    fn test_build_metadata_empty() {
        assert!(build_metadata("", "").is_none());
    }

    #[test]
    fn test_is_valid_url_accepts_http_and_https() {
        assert!(is_valid_url("https://example.com/webhook"));
        assert!(is_valid_url("http://localhost:3000/callback"));
        assert!(is_valid_url("https://api.example.com/v1/notify?token=abc"));
        assert!(is_valid_url("http://192.168.1.1:8080/hook"));
        assert!(is_valid_url(
            "https://example.com/path/to/resource#fragment"
        ));
    }

    #[test]
    fn test_is_valid_url_rejects_non_http_schemes() {
        assert!(!is_valid_url("ftp://files.example.com"));
        assert!(!is_valid_url("ws://example.com/ws"));
        assert!(!is_valid_url("wss://example.com/ws"));
        assert!(!is_valid_url("javascript:alert(1)"));
        assert!(!is_valid_url("data:text/html,<h1>hi</h1>"));
        assert!(!is_valid_url("file:///etc/passwd"));
    }

    #[test]
    fn test_is_valid_url_rejects_invalid_input() {
        assert!(!is_valid_url(""));
        assert!(!is_valid_url("not-a-url"));
        assert!(!is_valid_url("example.com"));
        assert!(!is_valid_url(" https://example.com"));
        assert!(!is_valid_url("http://"));
        assert!(!is_valid_url("https://"));
    }

    #[test]
    fn test_is_valid_url_accepts_case_insensitive_scheme() {
        assert!(is_valid_url("HTTP://EXAMPLE.COM"));
        assert!(is_valid_url("Https://Example.Com/path"));
    }
}
