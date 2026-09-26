//! Error-telemetry scrubbing for a Sentry-protocol error telemetry backend.
//!
//! random.cash is a crypto payment processor, so error payloads must **never**
//! carry secrets or customer data: wallet/private keys, mnemonics, API keys,
//! JWTs, bearer tokens, emails, on-chain addresses/hashes, HTTP request bodies,
//! or per-user identity. [`scrub_event`] is installed as the Sentry
//! `before_send` hook in every binary that initialises Sentry, [`scrub_log`]
//! as `before_send_log` for the separate structured-logs pipeline, and
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
use sentry::protocol::{Event, Log, Value};

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

/// Sentry `before_send_log` hook: strip PII/secrets before a structured log
/// record leaves the process.
///
/// Structured logs are a separate Sentry pipeline from events/breadcrumbs —
/// `before_send` above never sees them — so without this hook a log line
/// that interpolated a mnemonic, a JWT, or an on-chain address would ship
/// unredacted. Mirrors [`scrub_event`]: `body` is the free-text message,
/// `attributes` is the structured-data map, redacted the same way `extra` is.
#[must_use]
pub fn scrub_log(mut log: Log) -> Option<Log> {
    log.body = redact_secrets(&log.body);
    for (key, attribute) in log.attributes.iter_mut() {
        if is_sensitive_key(key) {
            attribute.0 = Value::String("[redacted]".to_string());
        } else {
            redact_value(&mut attribute.0);
        }
    }
    Some(log)
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

/// Builds the `ClientOptions` [`init_sentry`] hands to `sentry::init`. Split
/// out so a test can construct the exact options production uses — via
/// `sentry::test::with_captured_envelopes_options` — instead of hand-rolling
/// a lookalike `ClientOptions` that could quietly drift from what actually
/// ships.
fn client_options(
    dsn: Option<sentry::types::Dsn>,
    release: Option<Cow<'static, str>>,
    environment: String,
) -> sentry::ClientOptions {
    sentry::ClientOptions {
        dsn,
        release,
        environment: Some(Cow::Owned(environment)),
        // Never attach default PII (IP, cookies, request bodies). This is a
        // payment processor.
        send_default_pii: false,
        // Mandatory secret/PII scrubber: redacts wallet keys, mnemonics, JWTs,
        // API keys, emails and on-chain addresses before events leave the host.
        before_send: Some(Arc::new(scrub_event)),
        // Structured logs (see `sentry_log_event_filter` for which levels
        // actually reach this). Same mandatory scrubber, via the separate
        // hook logs go through.
        enable_logs: true,
        before_send_log: Some(Arc::new(scrub_log)),
        ..Default::default()
    }
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
    let guard = sentry::init(client_options(dsn, release, environment.clone()));
    (guard, dsn_configured, environment)
}

/// Env var controlling which tracing levels become Sentry *structured logs*,
/// independently of `LOG_LEVEL` (which controls what the process emits at
/// all, e.g. to stdout/Loki). Unset or unparsable resolves to `WARN`, the
/// stricter option, so a deploy that forgets to set this does not start
/// billing/shipping `mainnet` INFO logs to a third party by default; testnet
/// sets this to `info` explicitly to get the noisier feed.
#[must_use]
pub fn resolve_sentry_log_level() -> tracing::Level {
    match std::env::var("SENTRY_LOG_LEVEL") {
        Err(_) => tracing::Level::WARN,
        Ok(value) => value.parse().unwrap_or_else(|_| {
            // Unlike an unset var, this is a misconfiguration: someone set
            // the knob and got it wrong, so the fallback to WARN should be
            // visible rather than indistinguishable from "correctly set to
            // WARN".
            tracing::warn!(
                value = %value,
                "SENTRY_LOG_LEVEL is set but not a valid tracing level; defaulting to WARN"
            );
            tracing::Level::WARN
        }),
    }
}

/// Drops the `Log` flag from `filter` when `event_level` is more verbose than
/// `min_level`. Split out from [`sentry_log_event_filter`] so the threshold
/// logic is testable without constructing a real `tracing::Metadata`.
fn apply_log_level_gate(
    filter: sentry_tracing::EventFilter,
    event_level: tracing::Level,
    min_level: tracing::Level,
) -> sentry_tracing::EventFilter {
    if event_level > min_level {
        filter.difference(sentry_tracing::EventFilter::Log)
    } else {
        filter
    }
}

/// Wraps [`sentry_tracing::default_event_filter`], additionally dropping the
/// `Log` flag for any record more verbose than `min_level` — the knob behind
/// [`resolve_sentry_log_level`]. Breadcrumbs and error events are untouched:
/// this only changes whether a record also becomes a Sentry structured log.
pub fn sentry_log_event_filter(
    min_level: tracing::Level,
) -> impl Fn(&tracing::Metadata<'_>) -> sentry_tracing::EventFilter + Send + Sync + 'static {
    move |metadata| {
        apply_log_level_gate(
            sentry_tracing::default_event_filter(metadata),
            *metadata.level(),
            min_level,
        )
    }
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
mod tests;
#[cfg(test)]
mod reporting_tests;

