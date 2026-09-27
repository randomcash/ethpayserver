//! `sentry_event_filter`'s own tests: proves the `alloy_transport_ws` demotion
//! only ever swallows the target it exists for, never a target it shouldn't.
//!
//! A grandchild of `telemetry` for the same reason as `capture_tests`:
//! `telemetry.rs` has no room for another `mod` declaration, so this is
//! declared from `tests` instead.

use std::sync::{Arc, Mutex};

use crate::telemetry::*;

/// Captures the [`sentry_tracing::EventFilter`] `sentry_event_filter` assigns
/// to the next event recorded while `f` runs, by installing it as a real
/// tracing layer rather than hand-building a `Metadata`. `min_level` mirrors
/// the `SENTRY_LOG_LEVEL` knob each binary passes at its own call site; these
/// tests only ever assert on `EventFilter::Event`/`Breadcrumb`, which
/// `min_level` cannot change, so `tracing::Level::WARN` (the default) is fine
/// for all of them.
fn observed_filter(f: impl FnOnce()) -> sentry_tracing::EventFilter {
    use tracing_subscriber::layer::SubscriberExt;

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Option<sentry_tracing::EventFilter>>>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Capture {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let filter = sentry_event_filter(tracing::Level::WARN);
            *self.0.lock().expect("lock poisoned") = Some(filter(event.metadata()));
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
/// attempt itself* fails outright (DNS, refused, TLS) rather than completing
/// and then flapping, `alloy_pubsub`'s service loop does exhaust its retries
/// and logs its own `error!` under the `alloy_pubsub` target, not
/// `alloy_transport_ws` - so demoting the latter to a breadcrumb does not
/// hide that failure either. `ws_pubsub_retry_escalation.rs` shows this does
/// *not* generalise to a connection that keeps completing its handshake and
/// then resetting - see `sentry_event_filter`'s doc comment for why that case
/// relies on `resubscribe_if_stalled` instead.
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
