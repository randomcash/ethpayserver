use super::*;

#[test]
fn redacts_eth_private_key_and_address() {
    let pk = "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    let addr = "0x71C7656EC7ab88b098defB751B7401B5f6d8976F";
    let out = redact_secrets(&format!("signing with {pk} to {addr}"));
    assert!(!out.contains("deadbeefdead"), "private key leaked: {out}");
    assert!(!out.contains("71C7656E"), "address leaked: {out}");
    assert!(out.contains("[redacted-hex]"));
}

#[test]
fn redacts_bare_64_hex_private_key() {
    let pk = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
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

fn test_log(body: &str) -> Log {
    Log {
        level: sentry::protocol::LogLevel::Info,
        body: body.to_string(),
        trace_id: None,
        timestamp: std::time::SystemTime::now(),
        severity_number: None,
        attributes: Default::default(),
    }
}

#[test]
fn scrub_log_redacts_secret_shaped_body() {
    let pk = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    let log = test_log(&format!("loaded key {pk}"));

    let scrubbed = scrub_log(log).expect("log passes through");
    assert!(!scrubbed.body.contains(pk), "key leaked: {}", scrubbed.body);
}

#[test]
fn scrub_log_redacts_sensitive_keyed_attribute_regardless_of_shape() {
    use sentry::protocol::LogAttribute;

    let mut log = test_log("connecting");
    log.attributes.insert(
        "mnemonic".to_string(),
        LogAttribute(Value::String("not-shaped-like-a-secret".to_string())),
    );

    let scrubbed = scrub_log(log).expect("log passes through");
    let value = &scrubbed.attributes.get("mnemonic").unwrap().0;
    assert_eq!(value, &Value::String("[redacted]".to_string()));
}

#[test]
fn scrub_log_redacts_secret_shaped_attribute_under_an_innocuous_key() {
    use sentry::protocol::LogAttribute;

    let mut log = test_log("connecting");
    log.attributes.insert(
        "context".to_string(),
        LogAttribute(Value::String("token=sk_live_supersecret".to_string())),
    );

    let scrubbed = scrub_log(log).expect("log passes through");
    let value = &scrubbed.attributes.get("context").unwrap().0;
    assert!(
        !format!("{value:?}").contains("supersecret"),
        "attribute leaked: {value:?}"
    );
}

#[test]
fn scrub_log_leaves_benign_attributes_unredacted() {
    use sentry::protocol::LogAttribute;

    let mut log = test_log("connecting");
    log.attributes.insert(
        "retry_count".to_string(),
        LogAttribute(Value::Number(3.into())),
    );
    log.attributes.insert(
        "status".to_string(),
        LogAttribute(Value::String("connected".to_string())),
    );

    let scrubbed = scrub_log(log).expect("log passes through");
    assert_eq!(
        scrubbed.attributes.get("retry_count").unwrap().0,
        Value::Number(3.into()),
        "benign attribute should survive scrub_log unchanged"
    );
    assert_eq!(
        scrubbed.attributes.get("status").unwrap().0,
        Value::String("connected".to_string()),
        "benign attribute should survive scrub_log unchanged"
    );
}

/// Collects the `Log` items out of a batch of captured envelopes.
///
/// `pub(super)`: `reporting_tests` (telemetry's other test child module) needs
/// it too, and sibling modules don't share private items the way a parent and
/// child do.
pub(super) fn captured_logs(envelopes: &[sentry::Envelope]) -> Vec<Log> {
    envelopes
        .iter()
        .flat_map(|envelope| envelope.items())
        .filter_map(|item| match item {
            sentry::protocol::EnvelopeItem::ItemContainer(
                sentry::protocol::ItemContainer::Logs(logs),
            ) => Some(logs.iter().cloned()),
            _ => None,
        })
        .flatten()
        .collect()
}

/// Proves the wiring, not just the pure function: emits a `tracing::info!`
/// carrying a secret through the exact layer construction `server.rs` and
/// `evmmonitor/main.rs` use — `sentry_tracing::layer().event_filter(
/// sentry_log_event_filter(min_level))` — and a real `sentry::Client`
/// built from [`client_options`], the same function [`init_sentry`]
/// calls, so a later edit that drops `enable_logs`/`before_send_log`
/// from production breaks this test too. `scrub_log_redacts_secret_shaped_body`
/// above calls `scrub_log` directly, which proves the function redacts
/// but not that the SDK actually routes logs through it before sending —
/// this is the end-to-end check the ticket's "Check before shipping"
/// section asked for.
#[test]
fn scrub_log_redacts_a_secret_through_the_real_capture_pipeline() {
    use tracing_subscriber::prelude::*;

    let _dispatcher = tracing_subscriber::registry()
        .with(sentry_tracing::layer().event_filter(sentry_log_event_filter(tracing::Level::INFO)))
        .set_default();

    let pk = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

    let envelopes = sentry::test::with_captured_envelopes_options(
        || {
            tracing::info!("loaded key {pk}");
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
            !log.body.contains(pk),
            "key survived the real subscriber -> sentry_tracing -> \
             before_send_log pipeline: {}",
            log.body
        );
    }
}
