//! The scrubber driven through the real capture pipeline.
//!
//! Its sibling module builds a `Log` by hand and calls `scrub_log` directly,
//! which proves the redaction logic but not that a real field ever arrives in
//! the shape that logic expects. These drive a secret through the actual
//! subscriber layer and the real envelope capture instead - a sensitive key, a
//! secret-shaped value, and a debug-formatted value, each of which reaches the
//! scrubber by a different route.
//!
//! Every one asserts `!logs.is_empty()` before asserting redaction. Without
//! that, a run where no log reached the envelope would iterate an empty
//! collection, assert nothing, and pass while proving the opposite of its name.
//!
//! A grandchild of `telemetry` rather than a sibling: `telemetry.rs` is over the
//! line limit, so it cannot take another `mod` declaration, and both existing
//! test modules would cross the limit if these were appended to them. Declared
//! from `tests` instead, which has room.

use super::captured_logs;
use crate::telemetry::*;

/// The test above only proves the **body** path: a secret interpolated
/// into the format string. The far more common style in this codebase is
/// a structured field (`tracing::info!(mnemonic = %m, "...")`), which
/// `sentry_tracing` converts into `Log.attributes` through its own
/// `FieldVisitor` — code this crate does not own and cannot assume the
/// shape of. Every other attribute test in this file builds a `Log` by
/// hand and calls `scrub_log` directly, which proves the redaction logic
/// but not that a real field ever reaches it in the shape that logic
/// expects. This drives a sensitive **key** with an innocuous-looking
/// value through the real layer, the same way the test above drives the
/// body.
///
/// Ablated to confirm this can actually fail: with the `is_sensitive_key`
/// branch removed from `scrub_log`, this test panics with
/// `mnemonic field survived as a structured-log attribute through the
/// real sentry_tracing conversion: {"mnemonic": LogAttribute(String("just
/// some ordinary text about a wallet load")), ...}` — the plaintext value
/// present, unredacted, under the same key `contains_key` just confirmed
/// arrived. Restored before committing.
///
/// The value is deliberately *not* a real BIP-39 phrase: a genuine 12+
/// lowercase-word mnemonic is itself content-matched by the generic
/// mnemonic rule inside `redact_secrets` (the same one `redact_value`
/// falls through to for every non-sensitive key), so it would still come
/// out redacted even with `is_sensitive_key` broken — passing for the
/// wrong reason and proving nothing about the key-based branch this test
/// exists to isolate.
#[test]
fn scrub_log_redacts_a_sensitive_key_attribute_through_the_real_capture_pipeline() {
    use tracing_subscriber::prelude::*;

    let _dispatcher = tracing_subscriber::registry()
        .with(sentry_tracing::layer().event_filter(sentry_log_event_filter(tracing::Level::INFO)))
        .set_default();

    let m = "just some ordinary text about a wallet load";

    let envelopes = sentry::test::with_captured_envelopes_options(
        || {
            tracing::info!(mnemonic = %m, "loaded wallet");
        },
        client_options(None, None, "test".to_string()),
    );

    let logs = captured_logs(&envelopes);
    assert!(
        !logs.is_empty(),
        "expected at least one structured log to reach the envelope"
    );
    for log in &logs {
        assert!(
            log.attributes.contains_key("mnemonic"),
            "the mnemonic field never reached log.attributes at all — the \
             negative check below would pass vacuously regardless of \
             whether scrub_log redacts anything: {:?}",
            log.attributes
        );
        let attrs = format!("{:?}", log.attributes);
        assert!(
            !attrs.contains(m),
            "mnemonic field survived as a structured-log attribute through the \
             real sentry_tracing conversion: {attrs}"
        );
    }
}

/// Companion to the key-shaped case above: an innocuous key whose *value*
/// is secret-shaped (a bare hex private key), which only the value-side
/// `redact_value`/`redact_secrets` path — not the sensitive-key
/// allowlist — can catch.
///
/// Ablated to confirm this can actually fail: with the `redact_value`
/// branch removed from `scrub_log`, this test panics with `secret-shaped
/// value under an innocuous key survived as a structured-log attribute
/// through the real sentry_tracing conversion: {"context":
/// LogAttribute(String("deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef
/// deadbeefdeadbeef")), ...}` — the bare hex key present, unredacted, in
/// the value `contains_key` just confirmed arrived under. Restored
/// before committing.
#[test]
fn scrub_log_redacts_a_secret_shaped_attribute_value_through_the_real_capture_pipeline() {
    use tracing_subscriber::prelude::*;

    let _dispatcher = tracing_subscriber::registry()
        .with(sentry_tracing::layer().event_filter(sentry_log_event_filter(tracing::Level::INFO)))
        .set_default();

    let pk = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

    let envelopes = sentry::test::with_captured_envelopes_options(
        || {
            tracing::info!(context = %pk, "loaded wallet");
        },
        client_options(None, None, "test".to_string()),
    );

    let logs = captured_logs(&envelopes);
    assert!(
        !logs.is_empty(),
        "expected at least one structured log to reach the envelope"
    );
    for log in &logs {
        assert!(
            log.attributes.contains_key("context"),
            "the context field never reached log.attributes at all — the \
             negative check below would pass vacuously regardless of \
             whether scrub_log redacts anything: {:?}",
            log.attributes
        );
        let attrs = format!("{:?}", log.attributes);
        assert!(
            !attrs.contains(pk),
            "secret-shaped value under an innocuous key survived as a \
             structured-log attribute through the real sentry_tracing \
             conversion: {attrs}"
        );
    }
}

/// A matcher written against `&str` is the one most likely to miss a
/// value that is not a plain string. `sentry_tracing`'s `FieldVisitor`
/// calls `record_debug` for any `?field`, so a wrapped or
/// Debug-formatted value (quotes, an `Option` wrapper, nested
/// structure) is exactly the real shape that path produces — not a
/// hypothetical one.
///
/// Ablated to confirm this can actually fail: with the `redact_value`
/// branch removed from `scrub_log`, this test panics with
/// `debug-formatted/wrapped secret survived as a structured-log
/// attribute through the real sentry_tracing conversion: {"context":
/// LogAttribute(String("Some(\"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef
/// deadbeefdeadbeefdeadbeef\")")), ...}` — the wrapped key present,
/// unredacted, in the `Option`-wrapped Debug string `contains_key` just
/// confirmed arrived as. Restored before committing.
#[test]
fn scrub_log_redacts_a_debug_formatted_secret_attribute_through_the_real_capture_pipeline() {
    use tracing_subscriber::prelude::*;

    let _dispatcher = tracing_subscriber::registry()
        .with(sentry_tracing::layer().event_filter(sentry_log_event_filter(tracing::Level::INFO)))
        .set_default();

    let pk = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

    let envelopes = sentry::test::with_captured_envelopes_options(
        || {
            // `?` (Debug) wraps the value in `Some("...")` rather than
            // handing over a bare &str, matching the shape produced when
            // a caller logs a secret through a Debug-only type.
            tracing::info!(context = ?Some(pk), "loaded wallet");
        },
        client_options(None, None, "test".to_string()),
    );

    let logs = captured_logs(&envelopes);
    assert!(
        !logs.is_empty(),
        "expected at least one structured log to reach the envelope"
    );
    for log in &logs {
        assert!(
            log.attributes.contains_key("context"),
            "the context field never reached log.attributes at all — the \
             negative check below would pass vacuously regardless of \
             whether scrub_log redacts anything: {:?}",
            log.attributes
        );
        let attrs = format!("{:?}", log.attributes);
        assert!(
            !attrs.contains(pk),
            "debug-formatted/wrapped secret survived as a structured-log \
             attribute through the real sentry_tracing conversion: {attrs}"
        );
    }
}
