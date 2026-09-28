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
async fn resuming_past_the_retention_window_fails_out_of_range_without_changing_the_epoch() {
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

    for i in 0..250u8 {
        bridge
            .publish(&make_event(B256::from([i; 32])))
            .await
            .unwrap();
    }

    let epoch_before = bridge.current_epoch().await.unwrap();

    // seq 1 was the first entry published. `MAXLEN ~` only drops whole
    // stream nodes (up to 100 entries each by default), so 250 publishes
    // are needed to guarantee the first node is released whatever the
    // entry size; with a maxlen of 3 seq 1 is then long gone.
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
    assert_eq!(
        epoch_before, epoch_after,
        "bumping the epoch would turn the next start into a silent resume past the gap"
    );
}

/// The retention check in `subscribe_from` only runs once, before the
/// stream starts - it does not cover a reader that falls behind *while
/// already subscribed*. A consumer that stalls applying one envelope for
/// long enough lets `XADD ... MAXLEN ~` trim entries it has not read yet;
/// without a check on every batch, the next `XREAD` would just hand back
/// whatever survives past `last_id`, silently skipping the gap.
#[tokio::test]
#[ignore]
async fn a_gap_that_opens_while_already_subscribed_ends_the_stream() {
    let suffix = Uuid::new_v4();
    let bridge = RedisBridge::new_with_maxlen(
        &redis_url(),
        &format!("test:durable_resume:{suffix}:events"),
        &format!("test:durable_resume:{suffix}:commands"),
        3,
    )
    .await
    .expect("connect to REDIS_URL");

    for i in 0..3u8 {
        bridge
            .publish(&make_event(B256::from([i; 32])))
            .await
            .unwrap();
    }

    let mut stream = bridge.subscribe_from(None).await.unwrap();

    // Drain everything published so far, so the stream's internal position
    // is caught up to seq 3 before the reader "stalls".
    for expected_seq in 1..=3i64 {
        let envelope = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(envelope.cursor.seq, expected_seq);
    }

    // While the reader is stalled (not polling `.next()`), enough publishes
    // land to trim seq 4 and beyond out of the 3-entry retention window -
    // the same volume the sibling out-of-range test above needs: trimming
    // is approximate and works on whole nodes of up to 100 entries.
    for i in 3..253u8 {
        bridge
            .publish(&make_event(B256::from([i; 32])))
            .await
            .unwrap();
    }

    // Before the fix this silently resumed at whatever seq survived
    // trimming, skipping every entry between 4 and it with no error. It
    // must end the stream instead.
    let next = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next()).await;
    match next {
        Ok(Some(envelope)) => panic!(
            "expected the stream to end on a mid-subscription gap, got seq {}",
            envelope.cursor.seq
        ),
        Ok(None) => {}
        Err(_) => panic!("stream neither ended nor delivered an envelope within the timeout"),
    }
}

/// A total Redis data loss (restart with no AOF/RDB, an evicted keyspace)
/// must not let the outbox mint the same epoch it had before. If it did, a
/// cursor persisted before the loss would compare equal to the "fresh" one
/// and be silently trusted to resume from a `seq` the new, reset outbox can
/// never reach - the exact silent-loss failure this whole mechanism exists
/// to close, reached through its own bootstrap.
#[tokio::test]
#[ignore]
async fn a_full_keyspace_loss_mints_an_epoch_that_does_not_collide_with_the_old_one() {
    let suffix = Uuid::new_v4();
    let events_channel = format!("test:durable_resume:{suffix}:events");
    let commands_channel = format!("test:durable_resume:{suffix}:commands");
    let bridge = RedisBridge::new(&redis_url(), &events_channel, &commands_channel)
        .await
        .expect("connect to REDIS_URL");

    bridge
        .publish(&make_event(B256::from([1u8; 32])))
        .await
        .unwrap();
    let epoch_before = bridge.current_epoch().await.unwrap();

    // Simulate total data loss for this outbox: delete every key it owns,
    // the same effect a Redis restart with no persistence would have.
    let client = redis::Client::open(redis_url()).expect("connect to REDIS_URL");
    let mut conn = client
        .get_multiplexed_async_connection()
        .await
        .expect("raw connection");
    let _: () = redis::AsyncCommands::del(
        &mut conn,
        vec![
            events_channel.clone(),
            format!("{events_channel}:seq"),
            format!("{events_channel}:epoch"),
        ],
    )
    .await
    .expect("DEL outbox keys");

    // A fresh bridge instance stands in for the process that reconnects
    // after the loss - the point is that nothing else survived to hand it
    // the old epoch back.
    let reconnected = RedisBridge::new(&redis_url(), &events_channel, &commands_channel)
        .await
        .expect("connect to REDIS_URL");
    let epoch_after = reconnected.current_epoch().await.unwrap();

    assert_ne!(
        epoch_before, epoch_after,
        "a fresh mint after total data loss collided with the pre-loss epoch - a persisted \
         cursor from before the loss would be silently trusted to resume into the reset outbox"
    );
}
