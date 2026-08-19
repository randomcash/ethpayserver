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
            // RPC provider URLs carry the API key in the path (Alchemy
            // `/v2/<key>`, Infura `/v3/<key>`, QuickNode `/<token>/`), which no
            // `key=value` rule can see. Redact any path segment long enough to
            // be a credential and keep scheme/host, so reports still say which
            // provider failed. Query-string forms (`?api-key=`) are already
            // covered by the `key: value` rule below. The 16-char floor clears
            // real path words (`market_chart`, `getting-started`) while still
            // catching short provider keys — checked against live Alchemy,
            // Infura, QuickNode, CoinGecko, Etherscan and Kraken URL shapes.
            (
                build(
                    r"((?:https?|wss?)://[A-Za-z0-9.\-]+(?::[0-9]+)?(?:/[A-Za-z0-9._\-]{1,15})*/)[A-Za-z0-9._\-]{16,}",
                ),
                "${1}[redacted-rpc-key]",
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
        Value::Object(map) => redact_map(map.iter_mut()),
        _ => {}
    }
}

/// Redact a structured-data map, keyed.
///
/// The text rules need key and value in one string (`password = hunter2`). In a
/// map they are separate, so a bare "hunter2" matches nothing — the key is the
/// only signal there is. Every map on an event goes through here (`extra`,
/// breadcrumb data, stack-frame vars), which is exactly where that shape shows
/// up.
fn redact_map<'a>(entries: impl IntoIterator<Item = (&'a String, &'a mut Value)>) {
    for (key, value) in entries {
        if is_sensitive_key(key) {
            *value = Value::String("[redacted]".to_string());
        } else {
            redact_value(value);
        }
    }
}

/// Whether a structured-data key names something whose value is a secret.
fn is_sensitive_key(key: &str) -> bool {
    const SENSITIVE: &[&str] = &[
        "password",
        "passwd",
        "secret",
        "token",
        "apikey",
        "api_key",
        "api-key",
        "mnemonic",
        "seed",
        "privatekey",
        "private_key",
        "private-key",
        "authorization",
        "auth",
        "dsn",
        "credential",
        "session",
        "cookie",
    ];
    let key = key.to_ascii_lowercase();
    SENSITIVE.iter().any(|needle| key.contains(needle))
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
    for text in [
        &mut event.message,
        &mut event.culprit,
        &mut event.transaction,
        &mut event.logger,
    ]
    .into_iter()
    .flatten()
    {
        *text = redact_secrets(text);
    }
    if let Some(logentry) = event.logentry.as_mut() {
        logentry.message = redact_secrets(&logentry.message);
        // `message` is the template ("connecting to %s"); the values live in
        // `params`, so scrubbing only the message leaves the secret shipping.
        logentry.params.iter_mut().for_each(redact_value);
    }
    for exception in &mut event.exception.values {
        if let Some(value) = exception.value.as_mut() {
            *value = redact_secrets(value);
        }
        // Frame-local variables are captured verbatim where a backtrace
        // integration fills them in.
        if let Some(stacktrace) = exception.stacktrace.as_mut() {
            for frame in &mut stacktrace.frames {
                redact_map(frame.vars.iter_mut());
            }
        }
    }
    for breadcrumb in &mut event.breadcrumbs.values {
        if let Some(message) = breadcrumb.message.as_mut() {
            *message = redact_secrets(message);
        }
        redact_map(breadcrumb.data.iter_mut());
    }
    redact_map(event.extra.iter_mut());
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
    fn redacts_api_key_in_rpc_url_path() {
        for url in [
            "https://eth-sepolia.g.alchemy.com/v2/alch_EPqFizwy30wuSFY4ewmD-",
            "wss://eth-sepolia.g.alchemy.com/v2/alch_EPqFizwy30wuSFY4ewmD-",
            "https://mainnet.infura.io/v3/0123456789abcdef0123456789abcdef",
        ] {
            let out = redact_secrets(url);
            assert!(
                out.contains("[redacted-rpc-key]"),
                "not redacted: {url} -> {out}"
            );
            assert!(
                !out.contains("alch_EPqFizwy30wuSFY4ewmD-"),
                "key leaked: {out}"
            );
            assert!(!out.contains("0123456789abcdef"), "key leaked: {out}");
        }
    }

    #[test]
    fn redacts_rpc_key_inside_a_provider_error_string() {
        // The shape alloy produces when a connection fails.
        let msg = "error sending request for url \
                   (https://eth-sepolia.g.alchemy.com/v2/alch_EPqFizwy30wuSFY4ewmD-)";
        let out = redact_secrets(msg);
        assert!(
            !out.contains("alch_EPqFizwy30wuSFY4ewmD-"),
            "key leaked: {out}"
        );
        assert!(
            out.contains("eth-sepolia.g.alchemy.com"),
            "host should survive for diagnosis: {out}"
        );
    }

    #[test]
    fn keeps_keyless_rpc_urls_readable() {
        for url in [
            "https://eth.llamarpc.com",
            "https://polygon-rpc.com/",
            "http://192.168.1.10:8545/",
            "https://api.coingecko.com/api/v3/simple/price",
            "https://api.coingecko.com/api/v3/coins/ethereum/market_chart",
            "https://api.kraken.com/0/public/Ticker",
        ] {
            assert_eq!(redact_secrets(url), url, "over-redacted: {url}");
        }
    }

    #[test]
    fn scrub_event_redacts_logentry_params_and_frame_vars() {
        use sentry::protocol::{Exception, Frame, LogEntry, Stacktrace};

        let mut event = Event {
            logentry: Some(LogEntry {
                message: "connecting to %s".to_string(),
                params: vec![Value::String(
                    "https://eth-sepolia.g.alchemy.com/v2/alch_supersecretkey".to_string(),
                )],
            }),
            ..Default::default()
        };
        let mut frame = Frame::default();
        frame
            .vars
            .insert("password".to_string(), Value::String("hunter2".to_string()));
        event.exception.values.push(Exception {
            stacktrace: Some(Stacktrace {
                frames: vec![frame],
                ..Default::default()
            }),
            ..Default::default()
        });

        let scrubbed = scrub_event(event).expect("event passes through");
        let params = &scrubbed.logentry.as_ref().unwrap().params;
        assert!(
            !format!("{params:?}").contains("alch_supersecretkey"),
            "logentry param leaked: {params:?}"
        );
        let vars = &scrubbed.exception.values[0]
            .stacktrace
            .as_ref()
            .unwrap()
            .frames[0]
            .vars;
        assert!(
            !format!("{vars:?}").contains("hunter2"),
            "frame var leaked: {vars:?}"
        );
    }

    #[test]
    fn keeps_innocuous_text() {
        let msg = "failed to connect to database after 3 retries";
        assert_eq!(redact_secrets(msg), msg);
    }

    #[test]
    fn scrub_event_drops_request_user_and_server_name() {
        let event = Event {
            request: Some(sentry::protocol::Request::default()),
            user: Some(sentry::protocol::User::default()),
            server_name: Some("payserver-prod-01".into()),
            ..Default::default()
        };

        let scrubbed = scrub_event(event).expect("event passes through");
        assert!(scrubbed.request.is_none());
        assert!(scrubbed.user.is_none());
        assert!(scrubbed.server_name.is_none());
    }

    #[test]
    fn scrub_event_redacts_message_and_extra() {
        let mut event = Event {
            message: Some(
                "panic: invalid key \
                 0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318"
                    .to_string(),
            ),
            ..Default::default()
        };
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
