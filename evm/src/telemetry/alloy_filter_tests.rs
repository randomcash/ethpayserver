use super::*;
use std::sync::Mutex;

/// A `tracing::Subscriber` that runs [`sentry_event_filter`] on every
/// event it sees and records the verdict, so the filter can be tested
/// against real `tracing::Metadata` produced by the actual macros rather
/// than a hand-built one.
struct RecordingSubscriber(Arc<Mutex<Vec<sentry_tracing::EventFilter>>>);

impl tracing::Subscriber for RecordingSubscriber {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        self.0
            .lock()
            .unwrap()
            .push(sentry_event_filter(event.metadata()));
    }
    fn enter(&self, _span: &tracing::span::Id) {}
    fn exit(&self, _span: &tracing::span::Id) {}
}

/// `alloy_transport_ws` logs `error!` for a single WebSocket frame that
/// failed to parse — for example a provider sending a bare
/// `{"error": ...}` frame with no `id` over a block subscription, which
/// isn't a valid notification or response. That log comes from a read
/// loop that already reconnects and re-subscribes on its own; without
/// this filter it pages exactly like a real outage on every transient
/// bad frame. A real, unrecovered failure must still page: this crate's
/// own `evm::monitor::source::rpc` target is untouched.
#[test]
fn alloy_ws_frame_noise_is_a_breadcrumb_but_our_own_subscription_failure_still_pages() {
    let seen = Arc::new(Mutex::new(Vec::new()));

    tracing::subscriber::with_default(RecordingSubscriber(seen.clone()), || {
        tracing::error!(target: "alloy_transport_ws", "failed to deserialize message");
        tracing::error!(
            target: "evm::monitor::source::rpc",
            "WebSocket subscription ended"
        );
    });

    let seen = seen.lock().unwrap();
    assert_eq!(
        seen[0].bits(),
        sentry_tracing::EventFilter::Breadcrumb.bits(),
        "alloy's own transient frame error must not page: {seen:?}"
    );
    // `.contains(Event)` rather than exact equality: `default_event_filter`
    // also sets the `Log` bit for ERROR-level records now that structured
    // logs exist, which is orthogonal to whether this pages as an event.
    assert!(
        seen[1].contains(sentry_tracing::EventFilter::Event),
        "our own subscription-ended error must still page: {seen:?}"
    );
}

/// The audited call sites live in `alloy_transport_ws::native` — a
/// submodule, not the crate root — so the filter has to match on the
/// target *prefix*, not equality. This proves that distinction actually
/// matters: it fires `error!` under the submodule target and would still
/// pass if `sentry_event_filter` used `==` instead of `starts_with`
/// against the one target the other test exercises, but not against this
/// one.
#[test]
fn alloy_ws_submodule_targets_are_also_a_breadcrumb() {
    let seen = Arc::new(Mutex::new(Vec::new()));

    tracing::subscriber::with_default(RecordingSubscriber(seen.clone()), || {
        tracing::error!(target: "alloy_transport_ws::native", "WS server missed a pong");
    });

    let seen = seen.lock().unwrap();
    assert_eq!(
        seen.iter().map(|f| f.bits()).collect::<Vec<_>>(),
        [sentry_tracing::EventFilter::Breadcrumb.bits()],
        "a submodule target under alloy_transport_ws must also be treated as noise: {seen:?}"
    );
}

/// `sentry_event_filter` treats every `error!` from `alloy_transport_ws`
/// as retried-underneath noise, on the strength of an audit of that
/// crate's specific call sites — not on the message. `alloy` is pinned by
/// a caret, so `cargo update` can move `alloy-transport-ws` to a release
/// that audit never saw. This fails the moment that happens, instead of
/// silently trusting a comment that may no longer be true.
#[test]
fn alloy_transport_ws_pin_matches_the_audited_release() {
    /// The `alloy-transport-ws` release [`sentry_event_filter`]'s
    /// whole-target match was audited against.
    const AUDITED_ALLOY_TRANSPORT_WS_VERSION: &str = "1.8.3";

    let lock = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../Cargo.lock"));
    let matches: Vec<&str> = lock
        .split("\n\n")
        .filter(|pkg| pkg.contains("name = \"alloy-transport-ws\"\n"))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "Cargo.lock must resolve exactly one `alloy-transport-ws` entry, found {}. A \
         second resolved version means an unaudited release is also linked into the \
         binary, which this test cannot see past a single `find`.",
        matches.len()
    );
    let resolved = matches[0]
        .lines()
        .find(|l| l.starts_with("version = "))
        .and_then(|l| l.split('"').nth(1))
        .expect("alloy-transport-ws entry in Cargo.lock has no version field");
    assert_eq!(
        resolved, AUDITED_ALLOY_TRANSPORT_WS_VERSION,
        "alloy-transport-ws moved from the version sentry_event_filter's target match was \
         audited against ({AUDITED_ALLOY_TRANSPORT_WS_VERSION}) to {resolved}. Re-run that \
         audit against the new release's error!() call sites, then move \
         AUDITED_ALLOY_TRANSPORT_WS_VERSION forward."
    );
}
