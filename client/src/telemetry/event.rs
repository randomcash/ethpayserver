//! Sentry-protocol event and envelope construction.
//!
//! Everything the browser sends is assembled here, field by field, so the set
//! of things that *can* leave the page is auditable in one place: no cookies,
//! no request bodies, no user identity, no query strings — mirroring the
//! `send_default_pii = false` + `before_send` posture of the Rust binaries
//! (`evm::telemetry::scrub_event`). Every free-text field is passed through
//! [`redact_secrets`] here rather than at the call sites, so a new capture
//! source cannot forget to scrub.

use serde_json::json;

use super::scrub::redact_secrets;

/// One captured client-side failure.
pub struct Report<'a> {
    /// Capture source: `panic`, `error` or `unhandledrejection`.
    pub kind: &'a str,
    /// Human-readable failure text (panic message, `Error.message`, …).
    pub message: &'a str,
    /// Raw JS/WASM stack. errex has no source maps yet, so this ships verbatim
    /// (after redaction) and is read as-is.
    pub stack: Option<String>,
    /// Route the failure happened on — `location.pathname` only, never the
    /// query string or fragment.
    pub route: Option<String>,
}

/// Per-event data supplied by the caller (kept out of [`envelope`] so it stays
/// a pure function that unit tests can pin).
pub struct Meta {
    /// 32 lowercase hex characters.
    pub event_id: String,
    /// Unix seconds.
    pub timestamp: f64,
    /// Build identifier, when the bundle was built in CI.
    pub release: Option<String>,
    /// Deployment name (`testnet`, `dev`, …).
    pub environment: Option<String>,
}

/// Serialize a report as a newline-delimited Sentry envelope.
#[must_use]
pub fn envelope(report: &Report<'_>, meta: &Meta) -> String {
    let event = event(report, meta);
    let payload = event.to_string();
    let header = json!({ "event_id": meta.event_id });
    let item = json!({
        "type": "event",
        "content_type": "application/json",
        "length": payload.len(),
    });
    format!("{header}\n{item}\n{payload}\n")
}

/// Build the event body itself.
fn event(report: &Report<'_>, meta: &Meta) -> serde_json::Value {
    let mut event = json!({
        "event_id": meta.event_id,
        "timestamp": meta.timestamp,
        "platform": "javascript",
        "level": "error",
        "logger": "wasm-client",
        "sdk": { "name": env!("CARGO_PKG_NAME"), "version": env!("CARGO_PKG_VERSION") },
        "tags": { "component": "leptos-client", "capture": report.kind },
        "exception": { "values": [{
            "type": report.kind,
            "value": redact_secrets(report.message),
        }] },
    });

    // Optional fields are inserted rather than emitted as `null`, so an event
    // built outside CI (no release) or outside a route carries no empty keys.
    if let Some(map) = event.as_object_mut() {
        if let Some(release) = &meta.release {
            map.insert("release".into(), json!(release));
        }
        if let Some(environment) = &meta.environment {
            map.insert("environment".into(), json!(environment));
        }
        if let Some(route) = &report.route {
            map.insert("transaction".into(), json!(redact_secrets(route)));
        }
        if let Some(stack) = &report.stack {
            map.insert("extra".into(), json!({ "stack": redact_secrets(stack) }));
        }
    }
    event
}

#[cfg(test)]
mod tests {
    use super::{Meta, Report, envelope};

    fn meta() -> Meta {
        Meta {
            event_id: "0123456789abcdef0123456789abcdef".to_string(),
            timestamp: 1_755_600_000.0,
            release: Some("abc1234".to_string()),
            environment: Some("testnet".to_string()),
        }
    }

    fn parse(body: &str) -> (serde_json::Value, serde_json::Value, serde_json::Value) {
        let (header, item, payload) = split(body);
        (
            serde_json::from_str(header).expect("header json"),
            serde_json::from_str(item).expect("item json"),
            serde_json::from_str(payload).expect("payload json"),
        )
    }

    /// The three newline-delimited envelope lines, unparsed.
    fn split(body: &str) -> (&str, &str, &str) {
        let mut lines = body.lines();
        let header = lines.next().expect("header line");
        let item = lines.next().expect("item line");
        let payload = lines.next().expect("payload line");
        assert_eq!(lines.next(), None, "envelope should have exactly 3 lines");
        (header, item, payload)
    }

    #[test]
    fn builds_a_three_part_envelope_with_a_matching_length() {
        let body = envelope(
            &Report {
                kind: "panic",
                message: "index out of bounds",
                stack: None,
                route: None,
            },
            &meta(),
        );
        let (header, item, payload) = parse(&body);
        let (.., raw_payload) = split(&body);

        assert_eq!(header["event_id"], "0123456789abcdef0123456789abcdef");
        assert_eq!(item["type"], "event");
        assert_eq!(item["length"], raw_payload.len());
        assert_eq!(payload["platform"], "javascript");
        assert_eq!(payload["release"], "abc1234");
        assert_eq!(payload["environment"], "testnet");
        assert_eq!(payload["exception"]["values"][0]["type"], "panic");
        assert_eq!(
            payload["exception"]["values"][0]["value"],
            "index out of bounds"
        );
    }

    #[test]
    fn scrubs_every_free_text_field() {
        let body = envelope(
            &Report {
                kind: "error",
                message: "no wallet for 0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
                stack: Some(
                    "at fetch (Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJhIjoxfQ.sig)"
                        .to_string(),
                ),
                route: Some("/invoices/merchant@example.com".to_string()),
            },
            &meta(),
        );

        assert!(!body.contains("0x742d35Cc"), "{body}");
        assert!(!body.contains("eyJhbGciOiJIUzI1NiJ9"), "{body}");
        assert!(!body.contains("merchant@example.com"), "{body}");
        assert!(body.contains("[redacted-hex]"), "{body}");
    }

    #[test]
    fn never_carries_user_request_or_server_identity() {
        let body = envelope(
            &Report {
                kind: "unhandledrejection",
                message: "boom",
                stack: Some("at main".to_string()),
                route: Some("/dashboard".to_string()),
            },
            &meta(),
        );
        let (_, _, payload) = parse(&body);

        for forbidden in ["user", "request", "server_name", "breadcrumbs", "contexts"] {
            assert!(
                payload.get(forbidden).is_none(),
                "leaked {forbidden}: {payload}"
            );
        }
        assert_eq!(payload["transaction"], "/dashboard");
    }

    #[test]
    fn omits_absent_optional_fields() {
        let body = envelope(
            &Report {
                kind: "panic",
                message: "boom",
                stack: None,
                route: None,
            },
            &Meta {
                event_id: "0123456789abcdef0123456789abcdef".to_string(),
                timestamp: 0.0,
                release: None,
                environment: None,
            },
        );
        let (_, _, payload) = parse(&body);

        for absent in ["release", "environment", "transaction", "extra"] {
            assert!(payload.get(absent).is_none(), "{absent} should be omitted");
        }
    }
}
