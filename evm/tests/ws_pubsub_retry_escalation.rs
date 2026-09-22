//! Runs the real `alloy_pubsub`/`alloy_transport_ws` retry loop against a WS
//! server that resets every connection without a close handshake - the exact
//! "Connection reset without closing handshake" a Sentry-caught
//! `alloy_transport_ws::native` error reported - and checks what the real
//! crates actually log, rather than trusting a hand-built `Metadata`.
//!
//! This also documents a real finding: a connection that keeps completing
//! its WS handshake and then resetting immediately after never causes
//! `alloy_pubsub`'s retry loop to "give up" and log under its own target.
//! `reconnect_with_retries` only counts a `reconnect()` call as a failed
//! attempt if establishing the connection itself errors, and here it always
//! succeeds - the connection just dies again right after. So `max_retries`
//! is never approached, and `alloy_pubsub::service`'s give-up log is never
//! reached, no matter how long this runs. `evm::telemetry::sentry_event_filter`
//! does not rely on that log as its backstop for exactly this reason; see its
//! doc comment for the real one (`ChainMonitor::resubscribe_if_stalled`).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use alloy::providers::{ProviderBuilder, WsConnect};
use tokio::net::TcpListener;
use tracing_subscriber::layer::SubscriberExt;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<(String, sentry_tracing::EventFilter)>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Capture {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if *event.metadata().level() != tracing::Level::ERROR {
            return;
        }
        let filter = evm::telemetry::sentry_event_filter(event.metadata());
        self.0
            .lock()
            .expect("lock poisoned")
            .push((event.metadata().target().to_owned(), filter));
    }
}

// Runs on a single OS thread so the thread-local subscriber installed below
// also covers the tasks the WS client and `alloy_pubsub`'s retry loop spawn.
#[tokio::test(flavor = "current_thread")]
async fn a_flapping_ws_server_is_demoted_forever_and_never_self_escalates() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");

    // Complete the WS handshake, then drop the connection without sending a
    // close frame, on every attempt including retries.
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                break;
            };
            if let Ok(ws) = tokio_tungstenite::accept_async(socket).await {
                // Let the 101 response reach the client before resetting.
                tokio::time::sleep(Duration::from_millis(5)).await;
                drop(ws);
            }
        }
    });

    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    let _guard = tracing::subscriber::set_default(subscriber);

    let connect = WsConnect::new(format!("ws://{addr}"))
        .with_max_retries(2)
        .with_retry_interval(Duration::from_millis(20));

    // The first handshake succeeds before the server drops it, so this
    // resolves; the retry loop that follows runs in a background task.
    let _provider = ProviderBuilder::new()
        .disable_recommended_fillers()
        .connect_ws(connect)
        .await
        .expect("initial WS handshake succeeds before the reset");

    // Give the background retry loop plenty of cycles against the
    // always-resetting server - far more than the two retries configured
    // above, to prove it keeps going rather than genuinely exhausting them.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let events = capture.0.lock().expect("lock poisoned").clone();

    let transport_events: Vec<_> = events
        .iter()
        .filter(|(target, _)| target.starts_with("alloy_transport_ws"))
        .collect();
    assert!(
        transport_events.len() > 5,
        "expected the real alloy_transport_ws backend to log many resets \
         over half a second of a flapping connection, got {events:?}"
    );
    assert!(
        transport_events
            .iter()
            .all(|(_, filter)| !filter.contains(sentry_tracing::EventFilter::Event)),
        "a transient reset must not page on its own, got {events:?}"
    );

    // The real finding this test exists to pin down: a flapping connection
    // (handshake always succeeds, session always dies) never makes
    // `alloy_pubsub` log a give-up under its own target, so nothing here
    // ever resolves to `Event`. If a future `alloy` upgrade changes that
    // internal behavior, this assertion is what will tell us.
    assert!(
        events
            .iter()
            .all(|(_, filter)| !filter.contains(sentry_tracing::EventFilter::Event)),
        "alloy_pubsub was not expected to self-escalate against a flapping \
         connection - if it now does, sentry_event_filter's doc comment (and \
         the backstop it describes) needs re-checking, got {events:?}"
    );
}
