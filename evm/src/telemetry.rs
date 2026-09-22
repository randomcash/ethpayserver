//! Error-telemetry scrubbing for a Sentry-protocol error telemetry backend.
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
//!
//! # Relationship to the `scrub` crate
//!
//! The same rules exist a second time in payserver-commons `scrub`, as
//! hand-written scanners with no dependencies, because the browser client
//! needs them and `regex` costs ~1 MB of WASM. This regex table stays the
//! audited reference, and `parity_with_shared_scrubber` below asserts the two
//! agree across a corpus — so changing the rules here fails the build until
//! `scrub` follows. That check lives here because this is the only crate
//! where both implementations compile: `evm` pulls in alloy/sqlx and does not
//! build for `wasm32`, which is the whole reason there are two.

use std::borrow::Cow;
use std::sync::{Arc, OnceLock};

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

/// Whether error reporting is on, and — when it is off — whether that is
/// acceptable for the environment reporting failed to catch this itself once:
/// a disabled integration looks identical to a working one unless something
/// says so at boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportingStatus {
    /// A DSN is configured.
    Enabled,
    /// No DSN, but that's permitted in this (known-safe) environment.
    DisabledPermitted,
    /// No DSN in an environment that must not boot like this: `mainnet`,
    /// or anything unrecognised. An unset/unknown environment is the exact
    /// shape the incident that motivated this took — nothing said it was
    /// wrong — so it fails closed rather than being allowlisted as safe.
    DisabledRefused,
}

/// Decide whether error reporting is enabled and, if not, whether that's
/// permitted. Only `testnet` and `dev` are permitted to run disabled;
/// `mainnet` *and every other value, including unset*, are refused. Fails
/// closed: a typo'd or missing `SENTRY_ENVIRONMENT` must not be silently
/// treated as safe the way a missing `SENTRY_DSN` was.
///
/// Pure and side-effect free: the caller is responsible for logging the
/// result and, for [`ReportingStatus::DisabledRefused`], for refusing to
/// continue booting.
#[must_use]
pub fn reporting_status(dsn_configured: bool, environment: &str) -> ReportingStatus {
    if dsn_configured {
        ReportingStatus::Enabled
    } else if matches!(environment, "testnet" | "dev") {
        ReportingStatus::DisabledPermitted
    } else {
        ReportingStatus::DisabledRefused
    }
}

/// Resolve the `SENTRY_ENVIRONMENT` tag. A single source of truth so
/// [`init_sentry`] (what gets sent to Sentry) and
/// [`report_reporting_status`] (what a human sees at boot) cannot default it
/// two different ways and drift apart.
///
/// An unset variable resolves to the empty string, **not** to a permitted
/// value like `"dev"`. `reporting_status` already treats `""` the same as any
/// other unrecognised environment (refused). Defaulting it to `"dev"` here
/// used to silently launder "nobody set this" into "known safe to run
/// disabled" before `reporting_status` ever saw it — the exact failure shape
/// this ticket exists to close, one layer up.
#[must_use]
pub fn resolve_environment() -> String {
    std::env::var("SENTRY_ENVIRONMENT").unwrap_or_default()
}

/// Initialise Sentry from `SENTRY_DSN`, installing [`scrub_event`] as the
/// `before_send` hook and tagging events with [`resolve_environment`]. Shared
/// by the `server` and `evmmonitor` binaries so the mainnet boot-gate and the
/// PII scrubber live in exactly one place each, instead of two copies that
/// can quietly diverge.
///
/// Returns the init guard, whether a DSN was actually configured, and the
/// resolved environment tag — pass the latter two to
/// [`report_reporting_status`].
pub fn init_sentry(release: Option<Cow<'static, str>>) -> (sentry::ClientInitGuard, bool, String) {
    let dsn = std::env::var("SENTRY_DSN")
        .ok()
        .and_then(|s| s.parse().ok());
    let dsn_configured = dsn.is_some();
    let environment = resolve_environment();
    let guard = sentry::init(sentry::ClientOptions {
        dsn,
        release,
        environment: Some(Cow::Owned(environment.clone())),
        // Never attach default PII (IP, cookies, request bodies). This is a
        // payment processor.
        send_default_pii: false,
        // Mandatory secret/PII scrubber: redacts wallet keys, mnemonics, JWTs,
        // API keys, emails and on-chain addresses before events leave the host.
        before_send: Some(Arc::new(scrub_event)),
        ..Default::default()
    });
    (guard, dsn_configured, environment)
}

/// Sentry event filter for the `sentry_tracing` layer installed by the
/// `server` and `evmmonitor` binaries.
///
/// `alloy_transport_ws` logs at `error!` for every ordinary WebSocket hiccup a
/// long-lived RPC connection sees - a proxy resetting an idle socket, a
/// missed keepalive pong - and `sentry_tracing`'s default filter turns any
/// `error!` into a full Sentry event regardless of which crate logged it. So
/// every blip the library logs was paging as if nothing were handling it.
///
/// This filter only ever needs to swallow an *isolated* blip, never a
/// persistent failure, because it is not the backstop for a connection that
/// stays down: `ChainMonitor::resubscribe_if_stalled`
/// (`evm/src/monitor/chain/lifecycle.rs`) already watches for that on its own
/// clock, independent of anything `alloy_transport_ws` or `alloy_pubsub` logs
/// or doesn't log. It resubscribes and logs its own `error!` under
/// `evm::monitor::chain::lifecycle` - a target this filter never touches -
/// once a block stream has gone silent for `stall_timeout`. So a transient
/// reset stays a breadcrumb, and a connection that never recovers pages
/// within one stall window regardless of what the WS layer's own retry logic
/// happens to be doing underneath it. `our_own_errors_still_page` below
/// covers that target generically.
///
/// An earlier version of this filter argued instead that `alloy_pubsub`'s own
/// service loop (`alloy_pubsub::service`) always logs a paging `error!` when
/// *it* gives up retrying, and used that as the backstop. That turned out not
/// to hold in general: `evm/tests/ws_pubsub_retry_escalation.rs` runs the
/// real `alloy_pubsub`/`alloy_transport_ws` retry loop (pinned to `alloy =
/// "1.0"`, resolved in `Cargo.lock` to 1.8.3) against a WS server that
/// completes the handshake and then resets every connection, including
/// retries, and `alloy_pubsub::service` never logs at all - it just
/// reconnects, dies, and reconnects again, forever. Traced against that
/// pinned source: `reconnect_with_retries`
/// (`alloy-pubsub-1.8.3/src/service.rs:195`) only counts a `reconnect()` call
/// as a failed attempt if establishing the connection itself errors; a
/// connection that establishes fine and then dies immediately after counts as
/// a *successful* reconnect, so `max_retries` is never approached and the
/// give-up log at `service.rs:205` is never reached. That is a real gap in
/// `alloy_pubsub`, not a defect in this filter - it just means this filter
/// cannot lean on it, which is why the actual backstop is our own
/// `resubscribe_if_stalled` instead.
///
/// `alloy_pubsub_giving_up_still_pages` below documents the narrower case
/// where `alloy_pubsub`'s give-up log does still apply - the connection
/// attempt itself fails outright (DNS, refused, TLS) rather than flapping -
/// which still pages correctly since this filter never touches that target
/// either. It is not relied on as the general backstop.
///
/// Only `error!`-level `alloy_transport_ws` events are demoted: the noise
/// this exists to quiet is specifically the `error!` call sites in
/// `alloy_transport_ws::native`, not `debug!`/`trace!` chatter the same
/// target might log, so this filter does not touch those.
#[must_use]
pub fn sentry_event_filter(metadata: &tracing::Metadata) -> sentry_tracing::EventFilter {
    if *metadata.level() == tracing::Level::ERROR
        && metadata.target().starts_with("alloy_transport_ws")
    {
        return sentry_tracing::EventFilter::Breadcrumb;
    }
    sentry_tracing::default_event_filter(metadata)
}

/// Log whether error reporting is on, at INFO, always — never the DSN itself
/// — and refuse to continue when [`reporting_status`] says this environment
/// must not run disabled.
pub fn report_reporting_status(dsn_configured: bool, environment: &str) -> anyhow::Result<()> {
    let environment = if environment.is_empty() {
        "(unset)"
    } else {
        environment
    };
    match reporting_status(dsn_configured, environment) {
        ReportingStatus::Enabled => {
            tracing::info!(environment = %environment, "error reporting enabled");
        }
        ReportingStatus::DisabledPermitted => {
            tracing::info!(
                environment = %environment,
                "error reporting DISABLED (no DSN configured); permitted in this environment"
            );
        }
        ReportingStatus::DisabledRefused => {
            anyhow::bail!(
                "error reporting DISABLED (no DSN configured) in environment={environment}; refusing to start"
            );
        }
    }
    Ok(())
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

    /// Asserts the dependency-free scanners in payserver-commons `scrub` produce
    /// byte-identical output to the regex table above, across a corpus chosen to
    /// hit every rule plus the boundaries between them (rule ordering, adjacent
    /// matches, non-ASCII neighbours, empty and truncated inputs).
    ///
    /// This is the only place both can be compiled. If it fails, the two
    /// implementations have diverged and the browser is redacting differently
    /// from the servers — fix `scrub`, do not delete the case.
    #[test]
    fn parity_with_shared_scrubber() {
        const CORPUS: &[&str] = &[
            "",
            "plain message with no secrets",
            "failed to fetch /api/invoices/inv_001: HTTP 502",
            "auth failed for eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.abc-_123 (401)",
            "eyJ.a.b eyJa..b eyJa.b.c",
            "0x742d35Cc6634C0532925a3b844Bc454e4438f44e paid 0x1234",
            "tx 4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318 mined",
            "zz4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318zz",
            "GET https://eth-mainnet.g.alchemy.com/v2/9f8e7d6c5b4a3210zz failed",
            "wss://polygon-mainnet.infura.io:443/ws/v3/0123456789abcdefzz closed",
            "https://api.coingecko.com/api/v3/simple/price?ids=ethereum",
            "https://telemetry.example.com/api/random.cash/envelope/",
            "no store for merchant@example.com or a@b.co.uk or bad@b.c",
            "Authorization: Bearer rc_live_opaque123",
            "authorization=eyJhbGciOiJIUzI1NiJ9.eyJhIjoxfQ.sig; token: abc, secret = \"s3cr3t\"",
            "api-key: k1 apikey:k2 API_KEY = k3 private-key k4 passwd:\tk5",
            "secretariat: not a secret keyword",
            "seed abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            "twelve plain lowercase words here really do trip the mnemonic rule ok now",
            "façade ↔ merchant@example.com ↔ 0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
            "token",
            "token=",
        ];
        for input in CORPUS {
            assert_eq!(
                redact_secrets(input),
                scrub::redact_secrets(input),
                "shared `scrub` diverged from the audited regex table on: {input}"
            );
        }
    }

    #[test]
    fn no_dsn_is_disabled_but_permitted_outside_mainnet() {
        assert_eq!(
            reporting_status(false, "testnet"),
            ReportingStatus::DisabledPermitted
        );
        assert_eq!(
            reporting_status(false, "dev"),
            ReportingStatus::DisabledPermitted
        );
    }

    #[test]
    fn no_dsn_in_mainnet_is_refused() {
        assert_eq!(
            reporting_status(false, "mainnet"),
            ReportingStatus::DisabledRefused
        );
    }

    #[test]
    fn no_dsn_with_unset_or_unrecognised_environment_fails_closed() {
        // The exact shape of the incident this exists for: nothing said the
        // environment was wrong, so an absent or misspelled
        // `SENTRY_ENVIRONMENT` must not be treated as a known-safe one.
        for environment in ["", "mainet", "prod", "MAINNET"] {
            assert_eq!(
                reporting_status(false, environment),
                ReportingStatus::DisabledRefused,
                "environment={environment:?} should fail closed"
            );
        }
    }

    #[test]
    fn a_configured_dsn_flips_every_environment_to_enabled() {
        for environment in ["mainnet", "testnet", "dev"] {
            assert_eq!(
                reporting_status(true, environment),
                ReportingStatus::Enabled,
                "environment={environment}"
            );
        }
    }

    #[test]
    fn report_reporting_status_is_ok_when_permitted_and_err_when_refused() {
        assert!(report_reporting_status(true, "mainnet").is_ok());
        assert!(report_reporting_status(false, "testnet").is_ok());
        assert!(report_reporting_status(false, "dev").is_ok());
        assert!(report_reporting_status(false, "mainnet").is_err());
        assert!(report_reporting_status(false, "").is_err());
    }

    /// Captures the [`sentry_tracing::EventFilter`] `sentry_event_filter`
    /// assigns to the next event recorded while `f` runs, by installing it as
    /// a real tracing layer rather than hand-building a `Metadata`.
    fn observed_filter(f: impl FnOnce()) -> sentry_tracing::EventFilter {
        use tracing_subscriber::layer::SubscriberExt;

        #[derive(Clone, Default)]
        struct Capture(Arc<std::sync::Mutex<Option<sentry_tracing::EventFilter>>>);

        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Capture {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _ctx: tracing_subscriber::layer::Context<'_, S>,
            ) {
                *self.0.lock().expect("lock poisoned") =
                    Some(sentry_event_filter(event.metadata()));
            }
        }

        let capture = Capture::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());
        tracing::subscriber::with_default(subscriber, f);
        capture
            .0
            .lock()
            .expect("lock poisoned")
            .take()
            .expect("event was recorded")
    }

    #[test]
    fn ws_transport_reset_is_a_breadcrumb_not_a_page() {
        let filter = observed_filter(|| {
            tracing::error!(
                target: "alloy_transport_ws::native",
                "WebSocket protocol error: Connection reset without closing handshake"
            );
        });
        assert!(
            filter.contains(sentry_tracing::EventFilter::Breadcrumb),
            "expected a breadcrumb, got {filter:?}"
        );
        assert!(
            !filter.contains(sentry_tracing::EventFilter::Event),
            "a WS drop the monitor already resubscribes past should not page: got {filter:?}"
        );
    }

    /// `evm::monitor::chain::lifecycle` is `resubscribe_if_stalled`'s own
    /// target - the real backstop for a connection that never recovers, on a
    /// clock independent of the WS layer. This target is untouched by
    /// `sentry_event_filter`, so it pages regardless of what
    /// `alloy_transport_ws`/`alloy_pubsub` do or don't log underneath it.
    #[test]
    fn our_own_errors_still_page() {
        let filter = observed_filter(|| {
            tracing::error!(target: "evm::monitor::chain::lifecycle", "block stream error");
        });
        assert!(
            filter.contains(sentry_tracing::EventFilter::Event),
            "an error from our own code must still reach Sentry as an event, got {filter:?}"
        );
    }

    /// A narrower case than the general backstop above: when a WS *connection
    /// attempt itself* fails outright (DNS, refused, TLS) rather than
    /// completing and then flapping, `alloy_pubsub`'s service loop does
    /// exhaust its retries and logs its own `error!` under the `alloy_pubsub`
    /// target, not `alloy_transport_ws` - so demoting the latter to a
    /// breadcrumb does not hide that failure either.
    /// `ws_pubsub_retry_escalation.rs` shows this does *not* generalise to a
    /// connection that keeps completing its handshake and then resetting -
    /// see `sentry_event_filter`'s doc comment for why that case relies on
    /// `resubscribe_if_stalled` instead.
    #[test]
    fn alloy_pubsub_giving_up_still_pages() {
        let filter = observed_filter(|| {
            tracing::error!(
                target: "alloy_pubsub::service",
                "Reconnect failed after 10 attempts, shutting down: backend gone"
            );
        });
        assert!(
            filter.contains(sentry_tracing::EventFilter::Event),
            "a WS connection alloy_pubsub gave up reconnecting must still page, got {filter:?}"
        );
    }

    #[test]
    fn resolve_environment_does_not_default_an_absent_var_to_a_permitted_value() {
        // Owns SENTRY_ENVIRONMENT for the duration of the test and restores
        // whatever was there before, since this is a process-global var and
        // no other test touches it.
        let previous = std::env::var("SENTRY_ENVIRONMENT").ok();

        // SAFETY: no other test reads or writes SENTRY_ENVIRONMENT.
        unsafe {
            std::env::remove_var("SENTRY_ENVIRONMENT");
        }
        let absent = resolve_environment();
        assert_ne!(
            absent, "dev",
            "an absent SENTRY_ENVIRONMENT must not resolve to a permitted value"
        );
        assert_eq!(
            reporting_status(false, &absent),
            ReportingStatus::DisabledRefused,
            "the resolved value for an absent var must fail closed, not boot disabled on mainnet"
        );

        // SAFETY: see above.
        unsafe {
            std::env::set_var("SENTRY_ENVIRONMENT", "testnet");
        }
        assert_eq!(resolve_environment(), "testnet");

        // SAFETY: see above.
        unsafe {
            match &previous {
                Some(value) => std::env::set_var("SENTRY_ENVIRONMENT", value),
                None => std::env::remove_var("SENTRY_ENVIRONMENT"),
            }
        }
    }
}
