#![allow(clippy::unwrap_used, clippy::expect_used)]
//! `EventConsumer::run` has a second recovery path besides the epoch check
//! in `reconcile_cursors`: the outbox can report a stored position as
//! trimmed - `Err(EvmError::EventStreamOutOfRange)` from `subscribe_from`
//! itself - even when the epoch it was committed under still matches, since
//! retention trims independently of the epoch key. Every other consumer
//! test runs against an unbounded `MemoryBridge`, which can never take this
//! branch; this one gives the bridge a real retention cap so the retry loop
//! in `run` - re-arm watch_retry, clear cursors, resume with `None`, and
//! not loop a second time - is exercised through the real entry point
//! rather than assumed correct because it type-checks.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use data_service::InMemoryDataService;
use evm::monitor::bridge::{EventBridge, MemoryBridge};
use evm::monitor::events::{MonitorEvent, PaymentDetected};
use evm::{Address, B256, U256};
use types::{InvoiceId, PaymentReader, StoreId};

use super::helpers::{create_test_consumer, create_test_invoice};

fn make_payment_detected(invoice_id: &InvoiceId, amount: u64, tx_hash: B256) -> MonitorEvent {
    MonitorEvent::PaymentDetected(PaymentDetected {
        chain_id: 1,
        invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: Address::ZERO,
        amount: U256::from(amount),
        tx_hash,
        block_number: 12_345_678,
        block_hash: B256::ZERO,
        log_index: None,
        is_native: true,
        token_address: None,
        from_address: Address::repeat_byte(0xab),
        confirmations: 1,
        required_confirmations: 12,
        detected_at: Utc::now(),
    })
}

async fn wait_for_payment(ds: &InMemoryDataService, invoice_id: &InvoiceId) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let payments = PaymentReader::get_for_invoice(ds, invoice_id)
                .await
                .unwrap();
            if !payments.is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("payment was never credited after the out-of-range retry")
}

#[tokio::test]
async fn a_resume_position_trimmed_out_from_under_the_consumer_re_arms_watch_retry_and_recovers() {
    let ds = Arc::new(InMemoryDataService::new());
    // Retains only the last 2 entries - small enough that the events
    // published while the consumer is down below blow straight past it.
    let bridge = Arc::new(MemoryBridge::with_max_retained(2));
    let store_id = StoreId::new();

    let warm_invoice_id = InvoiceId::new();
    create_test_invoice(&ds, &warm_invoice_id, store_id).await;
    bridge
        .publish(&make_payment_detected(
            &warm_invoice_id,
            1,
            B256::from([1u8; 32]),
        ))
        .await
        .unwrap();

    let consumer1 = create_test_consumer(ds.clone(), bridge.clone());
    let task1 = tokio::spawn(consumer1.run());
    wait_for_payment(&ds, &warm_invoice_id).await;
    // Let the cursor commit that follows the apply actually land before the
    // kill below.
    tokio::time::sleep(Duration::from_millis(50)).await;
    task1.abort();
    let _ = task1.await;

    // Publish enough while nothing is subscribed to push the committed
    // cursor's position out of the retention window entirely - the epoch
    // never changes here, only retention does.
    let invoice_id = InvoiceId::new();
    create_test_invoice(&ds, &invoice_id, store_id).await;
    for i in 0..5u8 {
        bridge
            .publish(&make_payment_detected(
                &InvoiceId::new(),
                1,
                B256::from([10 + i; 32]),
            ))
            .await
            .unwrap();
    }
    bridge
        .publish(&make_payment_detected(
            &invoice_id,
            500_000_000_000_000_000,
            B256::from([2u8; 32]),
        ))
        .await
        .unwrap();

    // Restart: the stored cursor's epoch still matches, but `subscribe_from`
    // must reject it as trimmed rather than silently resuming from whatever
    // the outbox happens to retain now - and `run` must retry once, not die.
    let consumer2 = create_test_consumer(ds.clone(), bridge.clone());
    let task2 = tokio::spawn(consumer2.run());
    wait_for_payment(&ds, &invoice_id).await;
    task2.abort();
    let _ = task2.await;

    assert!(
        ds.watch_reset_calls() > 0,
        "a trimmed resume position must re-arm watch_retry the same as an epoch mismatch does"
    );

    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert_eq!(payments.len(), 1);
    assert_eq!(payments[0].amount, "500000000000000000");
}
