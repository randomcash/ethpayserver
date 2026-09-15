//! Recovery behavior for a chain monitor whose block stream has silently died.
//!
//! A dropped or half-open WebSocket doesn't always deliver a close frame or an
//! error - sometimes the subscription just stops yielding blocks, forever,
//! with nothing to log and nothing for `select!` to notice. Meanwhile the RPC
//! itself stays reachable, so a health check that asks it directly (as
//! `current_block` does) keeps reporting a live chain. These tests exercise
//! the watchdog that notices the mismatch and reconnects without a process
//! restart.

use std::sync::Arc;
use std::time::Duration;

use evm::monitor::{
    ChainMonitor, ChainMonitorConfig, MockBlockSource, MonitorEvent, SourceStatus, make_block,
};

const TEST_CHAIN_ID: u64 = 11155111;

fn test_chain_config() -> &'static evm::ChainConfig {
    evm::get_any_chain_config(TEST_CHAIN_ID).expect("Sepolia config exists")
}

fn fast_confirm_config() -> ChainMonitorConfig {
    ChainMonitorConfig {
        required_confirmations: 3,
        max_blocks_per_scan: 100,
        // Short interval so the stall watchdog (piggybacked on this timer)
        // fires quickly in the test.
        confirmation_check_interval_secs: 1,
        // Short so the stall watchdog fires quickly in the test; production
        // defaults to 120s.
        stall_timeout_secs: 1,
        monitor_native: true,
        monitor_erc20: true,
    }
}

#[tokio::test]
async fn stalled_block_stream_resubscribes_and_resumes_without_restart() {
    let source = MockBlockSource::new(TEST_CHAIN_ID);
    let test_source = source.clone(); // shared handle for injection/inspection

    let monitor = Arc::new(ChainMonitor::new(
        test_chain_config(),
        source,
        fast_confirm_config(),
    ));

    let mut event_rx = monitor.subscribe();
    let monitor_clone = monitor.clone();
    let _monitor_handle = tokio::spawn(async move { monitor_clone.start().await });

    // The block source's broadcast channel drops anything sent before a
    // subscriber exists, so wait for the monitor to actually be listening
    // before injecting the baseline block.
    let started = tokio::time::timeout(Duration::from_secs(2), event_rx.recv()).await;
    assert!(
        matches!(started, Ok(Ok(MonitorEvent::MonitorStarted { .. }))),
        "expected MonitorStarted"
    );

    // Baseline: monitor has processed block 100, chain head is also 100.
    test_source.push_block(make_block(100));
    tokio::time::sleep(Duration::from_millis(150)).await;

    let health = monitor.get_health().await;
    assert_eq!(health.last_processed_block, Some(100));
    assert!(health.is_healthy, "caught up to the head must read healthy");

    let subscribes_before_stall = test_source.subscribe_count();

    // Simulate a WS that has silently stopped delivering: the chain head
    // keeps moving (current_block is a live RPC call in the mock, just like
    // the real RpcBlockSource), but no new block notification ever arrives.
    test_source.set_block_number(130);

    // Give the confirmation-check tick time to notice the stall and
    // resubscribe.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let health = monitor.get_health().await;
    assert!(
        !health.is_healthy,
        "30 blocks behind while connected must not read healthy"
    );
    assert!(
        test_source.subscribe_count() > subscribes_before_stall,
        "a chain that is connected but not advancing must trigger a resubscribe"
    );

    // The resubscribed stream is live: pushing a new block resumes
    // processing with no restart of the monitor task.
    test_source.push_block(make_block(131));
    tokio::time::sleep(Duration::from_millis(150)).await;

    let health = monitor.get_health().await;
    assert_eq!(health.last_processed_block, Some(131));
    assert!(
        health.is_healthy,
        "processing a fresh block must recover health"
    );
}

#[tokio::test]
async fn killed_connection_goes_red_then_resumes_without_restart() {
    // Distinct from the stall above: here the connection itself is what
    // dies (`SourceStatus` moves to `Disconnected`), not just the block
    // stream while nominally still `Connected`. This is the ticket's own
    // ablation: kill the RPC endpoint, watch the health flag go red, restore
    // it, and confirm the monitor resumes on its own.
    let source = MockBlockSource::new(TEST_CHAIN_ID);
    let test_source = source.clone();

    let monitor = Arc::new(ChainMonitor::new(
        test_chain_config(),
        source,
        fast_confirm_config(),
    ));

    let mut event_rx = monitor.subscribe();
    let monitor_clone = monitor.clone();
    let _monitor_handle = tokio::spawn(async move { monitor_clone.start().await });

    let started = tokio::time::timeout(Duration::from_secs(2), event_rx.recv()).await;
    assert!(
        matches!(started, Ok(Ok(MonitorEvent::MonitorStarted { .. }))),
        "expected MonitorStarted"
    );

    // Baseline: connected and caught up.
    test_source.push_block(make_block(200));
    tokio::time::sleep(Duration::from_millis(150)).await;

    let health = monitor.get_health().await;
    assert_eq!(health.status, SourceStatus::Connected);
    assert_eq!(health.last_processed_block, Some(200));
    assert!(health.is_healthy);

    let subscribes_before_kill = test_source.subscribe_count();

    // Kill the RPC endpoint: sever the stream the running monitor is
    // holding (the way a dropped WebSocket would) and make every connection
    // attempt fail until restored.
    test_source.kill_connection();
    test_source.set_block_number(230); // the chain kept advancing while we couldn't see it

    // Give the watchdog time to notice the stall, try to resubscribe, and
    // have that attempt fail - which is what actually flips `status` to
    // `Disconnected` (the mock, like the real `RpcBlockSource`, only updates
    // status as a side effect of a subscribe attempt, never on its own).
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let health = monitor.get_health().await;
    assert_eq!(health.status, SourceStatus::Disconnected);
    assert!(
        !health.is_healthy,
        "a disconnected source must read unhealthy"
    );
    assert_eq!(
        health.last_processed_block,
        Some(200),
        "nothing can be processed while the connection is down"
    );

    // Restore the endpoint. Nothing but the watchdog's own retries can
    // notice - there is no separate reconnect loop - so recovery depends on
    // it continuing to try on every tick while genuinely disconnected.
    test_source.restore_connection();
    tokio::time::sleep(Duration::from_millis(1500)).await;

    assert!(
        test_source.subscribe_count() > subscribes_before_kill,
        "recovery from a killed connection must resubscribe, with no process restart"
    );

    // The resubscribed stream is live: pushing a new block resumes
    // processing.
    test_source.push_block(make_block(231));
    tokio::time::sleep(Duration::from_millis(150)).await;

    let health = monitor.get_health().await;
    assert_eq!(health.last_processed_block, Some(231));
    assert!(
        health.is_healthy,
        "processing a fresh block after reconnect must recover health"
    );
}

#[tokio::test]
async fn healthy_chain_never_resubscribes() {
    let source = MockBlockSource::new(TEST_CHAIN_ID);
    let test_source = source.clone();

    let monitor = Arc::new(ChainMonitor::new(
        test_chain_config(),
        source,
        fast_confirm_config(),
    ));

    let mut event_rx = monitor.subscribe();
    let monitor_clone = monitor.clone();
    let _monitor_handle = tokio::spawn(async move { monitor_clone.start().await });

    let started = tokio::time::timeout(Duration::from_secs(2), event_rx.recv()).await;
    assert!(
        matches!(started, Ok(Ok(MonitorEvent::MonitorStarted { .. }))),
        "expected MonitorStarted"
    );

    // A live chain keeps delivering. The watchdog asks whether the stream is
    // still producing blocks, not how far behind the head the monitor is, so
    // "healthy" has to be exercised as continued delivery rather than as one
    // block followed by silence - silence past the timeout is exactly what it
    // is supposed to act on.
    for height in 1..=8 {
        test_source.push_block(make_block(height));
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    // Several confirmation-check ticks have elapsed, each of them finding a
    // stream that delivered recently - the watchdog must not have reconnected.
    assert_eq!(
        test_source.subscribe_count(),
        1,
        "a stream that is still delivering blocks must not be torn down and resubscribed"
    );
}

/// Catching up is not stalling.
///
/// After a restart, or a brief outage, the monitor is a long way behind the
/// chain head and working through the backlog. Blocks are arriving perfectly
/// well; it simply has not caught up yet.
///
/// Keying the watchdog on `is_healthy` conflates that with a dead
/// subscription, because `is_healthy` is false whenever the monitor is more
/// than ten blocks behind. It would then resubscribe on *every* confirmation
/// tick for the whole of the catch-up — churning the provider's subscription
/// at precisely the moment the monitor can least afford it, and doing nothing
/// to help it catch up, since the backlog is not a delivery problem.
///
/// Goes red against a watchdog that asks about lag instead of liveness.
#[tokio::test]
async fn a_lagging_but_delivering_chain_is_not_resubscribed() {
    let source = MockBlockSource::new(TEST_CHAIN_ID);
    let test_source = source.clone();

    let monitor = Arc::new(ChainMonitor::new(
        test_chain_config(),
        source,
        fast_confirm_config(),
    ));

    let mut event_rx = monitor.subscribe();
    let monitor_clone = monitor.clone();
    let _monitor_handle = tokio::spawn(async move { monitor_clone.start().await });

    let started = tokio::time::timeout(Duration::from_secs(2), event_rx.recv()).await;
    assert!(
        matches!(started, Ok(Ok(MonitorEvent::MonitorStarted { .. }))),
        "expected MonitorStarted"
    );

    let subscribes_before = test_source.subscribe_count();

    // A monitor mid-catch-up: the head is far ahead of anything it has
    // processed, and blocks keep arriving the whole time. The stream is alive;
    // the monitor is simply behind.
    //
    // The head is re-asserted after each push because `push_block` advances
    // the mock's head to the block it delivered - without this the lag closes
    // and the state under test never exists.
    for height in 1..=8 {
        test_source.push_block(make_block(height));
        test_source.set_block_number(1000);
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    let health = monitor.get_health().await;
    assert!(
        !health.is_healthy,
        "precondition: this is the lagging state the old watchdog would have \
         acted on - if it reads healthy the test proves nothing"
    );
    assert_eq!(
        test_source.subscribe_count(),
        subscribes_before,
        "a monitor that is merely behind must not have its subscription torn \
         down; blocks were arriving throughout"
    );
}
