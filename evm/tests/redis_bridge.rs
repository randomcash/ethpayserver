//! Durable event outbox behavior against a real Redis.
//!
//! Require `REDIS_URL`. Run with:
//!   REDIS_URL="redis://127.0.0.1:6379" cargo test -p evm --features redis --test redis_bridge -- --ignored
#![cfg(feature = "redis")]

use evm::monitor::bridge::{EventBridge, EventCursor, RedisBridge};
use evm::monitor::events::{MonitorEvent, PaymentDetected};
use evm::{Address, B256, EvmError, U256};
use tokio_stream::StreamExt;
use uuid::Uuid;

fn redis_url() -> String {
    std::env::var("REDIS_URL").expect("REDIS_URL required")
}

fn make_event(tx_hash: B256) -> MonitorEvent {
    MonitorEvent::PaymentDetected(PaymentDetected {
        chain_id: 1,
        invoice_id: Uuid::new_v4(),
        payment_address: Address::ZERO,
        amount: U256::from(1),
        tx_hash,
        block_number: 100,
        block_hash: B256::ZERO,
        log_index: None,
        is_native: true,
        token_address: None,
        from_address: Address::ZERO,
        confirmations: 1,
        required_confirmations: 12,
        detected_at: chrono::Utc::now(),
    })
}

/// A fresh key prefix per test run, so concurrent runs (and reruns against
/// the same Redis) never share an outbox or its epoch/seq counters.
async fn fresh_bridge() -> RedisBridge {
    let suffix = Uuid::new_v4();
    RedisBridge::new(
        &redis_url(),
        &format!("test:durable_resume:{suffix}:events"),
        &format!("test:durable_resume:{suffix}:commands"),
    )
    .await
    .expect("connect to REDIS_URL")
}

#[tokio::test]
#[ignore]
async fn publishes_survive_a_late_subscriber() {
    let bridge = fresh_bridge().await;

    // Nobody is subscribed when this publishes - the property a durable
    // outbox has and plain PUBLISH/SUBSCRIBE does not.
    bridge
        .publish(&make_event(B256::from([1u8; 32])))
        .await
        .unwrap();

    let mut stream = bridge.subscribe_from(None).await.unwrap();
    let envelope = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(envelope.cursor.seq, 1);
    assert_eq!(envelope.chain_id, 1);
}

#[tokio::test]
#[ignore]
async fn resumes_strictly_after_the_given_cursor() {
    let bridge = fresh_bridge().await;

    bridge
        .publish(&make_event(B256::from([1u8; 32])))
        .await
        .unwrap(); // seq 1
    bridge
        .publish(&make_event(B256::from([2u8; 32])))
        .await
        .unwrap(); // seq 2
    bridge
        .publish(&make_event(B256::from([3u8; 32])))
        .await
        .unwrap(); // seq 3

    let epoch = bridge.current_epoch().await.unwrap();
    let mut stream = bridge
        .subscribe_from(Some(EventCursor {
            epoch,
            seq: 1,
            block_height: 0,
        }))
        .await
        .unwrap();

    let first = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.cursor.seq, 2);

    let second = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.cursor.seq, 3);
}

#[tokio::test]
#[ignore]
async fn epoch_is_stable_across_bridge_instances_on_the_same_outbox() {
    let suffix = Uuid::new_v4();
    let events_key = format!("test:durable_resume:{suffix}:events");
    let commands_key = format!("test:durable_resume:{suffix}:commands");

    let publisher = RedisBridge::new(&redis_url(), &events_key, &commands_key)
        .await
        .unwrap();
    publisher
        .publish(&make_event(B256::from([1u8; 32])))
        .await
        .unwrap();
    let epoch_a = publisher.current_epoch().await.unwrap();

    // A second bridge instance over the same keys (a second server process,
    // or the same process after a restart) must see the outbox's real
    // epoch, not mint its own - two epochs for one lineage would make every
    // resume look like a mismatch.
    let resumer = RedisBridge::new(&redis_url(), &events_key, &commands_key)
        .await
        .unwrap();
    let epoch_b = resumer.current_epoch().await.unwrap();

    assert_eq!(epoch_a, epoch_b);
}

#[tokio::test]
#[ignore]
async fn live_events_published_after_subscribing_are_still_delivered() {
    let bridge = fresh_bridge().await;

    let mut stream = bridge.subscribe_from(None).await.unwrap();

    let publisher = bridge; // same bridge, just naming the role at each call site
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        publisher
            .publish(&make_event(B256::from([9u8; 32])))
            .await
            .unwrap();
    });

    let envelope = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(envelope.cursor.seq, 1);
}

#[tokio::test]
#[ignore]
async fn resuming_past_the_retention_window_bumps_the_epoch_and_fails_out_of_range() {
    let suffix = Uuid::new_v4();
    // A tiny cap so trimming is provoked by a handful of publishes rather
    // than the real 200,000-entry default.
    let bridge = RedisBridge::new_with_maxlen(
        &redis_url(),
        &format!("test:durable_resume:{suffix}:events"),
        &format!("test:durable_resume:{suffix}:commands"),
        3,
    )
    .await
    .expect("connect to REDIS_URL");

    for i in 0..20u8 {
        bridge
            .publish(&make_event(B256::from([i; 32])))
            .await
            .unwrap();
    }

    let epoch_before = bridge.current_epoch().await.unwrap();

    // seq 1 was the first entry published; with a maxlen of 3 it has long
    // since been trimmed out by the time 20 more have landed.
    let result = bridge
        .subscribe_from(Some(EventCursor {
            epoch: epoch_before,
            seq: 1,
            block_height: 0,
        }))
        .await;

    match result {
        Err(EvmError::EventStreamOutOfRange(_)) => {}
        Err(e) => panic!("expected EventStreamOutOfRange, got a different error: {e}"),
        Ok(_) => panic!("expected EventStreamOutOfRange, got a stream"),
    }

    let epoch_after = bridge.current_epoch().await.unwrap();
    assert_ne!(
        epoch_before, epoch_after,
        "a retention gap must bump the epoch so every other resumer sees the break too"
    );
}
