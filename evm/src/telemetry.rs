//! Error-telemetry scrubbing for the self-hosted errex (Sentry-protocol) backend.
//!
//! random.cash is a crypto payment processor, so error payloads must **never**
//! carry secrets or customer data: wallet/private keys, mnemonics, API keys,
//! JWTs, bearer tokens, emails, on-chain addresses/hashes, HTTP request bodies,
//! or per-user identity. [`scrub_event`] is installed as the Sentry
//! `before_send` hook in every binary that initialises Sentry, and
//! [`redact_secrets`] redacts secret-shaped substrings from free-text fields.
//!
//! This lives in `evm` (rather than being duplicated per binary) so the
//! `server` and `evmmonitor` binaries share one audited implementation. It is
//! gated behind the `sentry-scrub` feature so the scrubber and its `sentry` /
//! `regex` dependencies are only compiled where Sentry is actually used.

use std::sync::OnceLock;

use regex::Regex;
use sentry::protocol::{Event, Value};

/// Ordered `(pattern, replacement)` redaction rules applied to every free-text
/// field. Compiled once and reused for the life of the process.
fn rules() -> &'static [(Regex, &'static str)] {
    static RULES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    RULES.get_or_init(|| {
        // `unwrap` is safe: these are constant, test-covered patterns.
        #[allow(clippy::unwrap_used)]
        let build = |p: &str| Regex::new(p).unwrap();
        vec![
            // JSON Web Tokens (header.payload.signature).
            (
                build(r"eyJ[A-Za-z0-9_=-]+\.[A-Za-z0-9_=-]+\.[A-Za-z0-9_=-]+"),
                "[redacted-jwt]",
            ),
            // 0x-prefixed hex of address length or longer: addresses (40),
            // private keys / tx hashes / block hashes (64), signatures (130).
            (build(r"0x[0-9a-fA-F]{40,}"), "[redacted-hex]"),
            // Bare 64-char hex (private keys / hashes without the 0x prefix).
            (build(r"\b[0-9a-fA-F]{64}\b"), "[redacted-hex]"),
            // BIP-39 mnemonics: 12+ consecutive lowercase words.
            (
                build(r"\b(?:[a-z]+\s+){11,}[a-z]+\b"),
                "[redacted-mnemonic]",
            ),
            // Email addresses (customer PII).
            (
                build(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}"),
                "[redacted-email]",
            ),
            // `key: value` / `key=value` for sensitive keys, plus `Bearer <tok>`.
            (
                build(
                    r#"(?i)\b(api[_-]?key|secret|password|passwd|token|mnemonic|seed|private[_-]?key|authorization|bearer)\b(\s*[:=]\s*|\s+)("?)[^\s,;"']+"#,
                ),
                "$1$2$3[redacted]",
            ),
        ]
    })
}

/// Redact secret-shaped substrings from a free-text string.
///
/// Defence-in-depth for any error/log text that may have interpolated a
/// secret. Patterns intentionally err on the side of over-redaction.
#[must_use]
pub fn redact_secrets(input: &str) -> String {
    let mut out = std::borrow::Cow::Borrowed(input);
    for (re, replacement) in rules() {
        if re.is_match(&out) {
            out = std::borrow::Cow::Owned(re.replace_all(&out, *replacement).into_owned());
        }
    }
    out.into_owned()
}

/// Recursively redact secrets inside a JSON value (used for `extra` / breadcrumb
/// `data` blobs).
fn redact_value(value: &mut Value) {
    match value {
        Value::String(s) => *s = redact_secrets(s),
        Value::Array(items) => items.iter_mut().for_each(redact_value),
        Value::Object(map) => map.values_mut().for_each(redact_value),
        _ => {}
    }
}

/// Sentry `before_send` hook: strip PII/secrets before an event leaves the
/// process. Returning `Some(event)` lets the (scrubbed) event through;
/// returning `None` would drop it entirely.
///
/// Drops whole high-risk containers (HTTP request, user identity, server name)
/// and redacts secret-shaped text from every remaining free-text field.
#[must_use]
pub fn scrub_event(mut event: Event<'static>) -> Option<Event<'static>> {
    // Drop entire containers that routinely hold secrets / PII. With
    // `send_default_pii = false` these are usually empty, but never assume.
    event.request = None; // HTTP method/url/headers/cookies/body
    event.user = None; // id / email / ip / username
    event.server_name = None; // host identity

    // Redact secret-shaped text from remaining free-text fields.
    for field in [
        &mut event.message,
        &mut event.culprit,
        &mut event.transaction,
        &mut event.logger,
    ] {
        if let Some(text) = field {
            *text = redact_secrets(text);
        }
    }
    if let Some(logentry) = event.logentry.as_mut() {
        logentry.message = redact_secrets(&logentry.message);
    }
    for exception in &mut event.exception.values {
        if let Some(value) = exception.value.as_mut() {
            *value = redact_secrets(value);
        }
    }
    for breadcrumb in &mut event.breadcrumbs.values {
        if let Some(message) = breadcrumb.message.as_mut() {
            *message = redact_secrets(message);
        }
        breadcrumb.data.values_mut().for_each(redact_value);
    }
    event.extra.values_mut().for_each(redact_value);
    for tag_value in event.tags.values_mut() {
        *tag_value = redact_secrets(tag_value);
    }

    Some(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_eth_private_key_and_address() {
        let pk = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
        let addr = "0x71C7656EC7ab88b098defB751B7401B5f6d8976F";
        let out = redact_secrets(&format!("signing with {pk} to {addr}"));
        assert!(!out.contains("4c0883a6"), "private key leaked: {out}");
        assert!(!out.contains("71C7656E"), "address leaked: {out}");
        assert!(out.contains("[redacted-hex]"));
    }

    #[test]
    fn redacts_bare_64_hex_private_key() {
        let pk = "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
        let out = redact_secrets(&format!("key={pk}"));
        assert!(!out.contains(pk), "bare key leaked: {out}");
    }

    #[test]
    fn redacts_jwt() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.dozjgNryP4J3jVmNHl0w5N";
        let out = redact_secrets(&format!("auth failed for {jwt}"));
        assert!(!out.contains("eyJ"), "jwt leaked: {out}");
        assert!(out.contains("[redacted-jwt]"));
    }

    #[test]
    fn redacts_mnemonic() {
        let m = "legal winner thank year wave sausage worth useful legal winner thank yellow";
        let out = redact_secrets(&format!("loaded wallet: {m}"));
        assert!(!out.contains("sausage"), "mnemonic leaked: {out}");
        assert!(out.contains("[redacted-mnemonic]"));
    }

    #[test]
    fn redacts_email() {
        let out = redact_secrets("customer alice@example.com paid invoice");
        assert!(!out.contains("alice@example.com"), "email leaked: {out}");
    }

    #[test]
    fn redacts_keyed_secrets() {
        for input in [
            "api_key=sk_live_abc123def456",
            "Authorization: Bearer abc.def.ghi",
            "password = hunter2",
            "mnemonic: somesecretvalue",
        ] {
            let out = redact_secrets(input);
            assert!(out.contains("[redacted]"), "not redacted: {input} -> {out}");
            assert!(!out.contains("hunter2") || !input.contains("hunter2"));
        }
        let out = redact_secrets("password = hunter2");
        assert!(!out.contains("hunter2"), "password leaked: {out}");
    }

    #[test]
    fn keeps_innocuous_text() {
        let msg = "failed to connect to database after 3 retries";
        assert_eq!(redact_secrets(msg), msg);
    }

    #[test]
    fn scrub_event_drops_request_user_and_server_name() {
        let mut event = Event::default();
        event.request = Some(sentry::protocol::Request::default());
        event.user = Some(sentry::protocol::User::default());
        event.server_name = Some("payserver-prod-01".into());

        let scrubbed = scrub_event(event).expect("event passes through");
        assert!(scrubbed.request.is_none());
        assert!(scrubbed.user.is_none());
        assert!(scrubbed.server_name.is_none());
    }

    #[test]
    fn scrub_event_redacts_message_and_extra() {
        let mut event = Event::default();
        event.message = Some(
            "panic: invalid key 0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318"
                .to_string(),
        );
        event.extra.insert(
            "ctx".to_string(),
            Value::String("token=sk_live_supersecret".to_string()),
        );

        let scrubbed = scrub_event(event).expect("event passes through");
        assert!(!scrubbed.message.unwrap().contains("4c0883a6"));
        let extra = scrubbed.extra.get("ctx").and_then(Value::as_str).unwrap();
        assert!(!extra.contains("supersecret"), "extra leaked: {extra}");
    }
}
