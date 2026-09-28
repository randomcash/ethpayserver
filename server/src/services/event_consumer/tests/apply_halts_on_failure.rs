#![allow(clippy::unwrap_used, clippy::expect_used)]
//! `apply_envelope` must not let the durable cursor advance past an event
//! that failed to apply. `cursors` holds one scalar `(epoch, seq)` per
//! chain, so if the run loop kept going after a `handle_event` error and a
//! *later* envelope on the same chain went on to apply and commit, that
//! commit would move the chain's persisted cursor past the failed one -
//! the dedup check in `apply_envelope` would then treat the failed envelope
//! as already applied on every future resume, and it would never be
//! redelivered. This is exactly the "payment gone for good" failure this
//! whole mechanism exists to close, except reached without any restart at
//! all: a single ordinary `handle_event` error, followed by an unrelated
//! envelope for the same chain succeeding, silently drops the first.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use data_service::{ChainCursorReader, InMemoryDataService};
use evm::monitor::bridge::{EventBridge, MemoryBridge};
use evm::monitor::events::{MonitorEvent, PaymentDetected};
use evm::{Address, B256, U256};
use types::{InvoiceId, PaymentReader, StoreId};

use super::super::ADAPTER_ID;
use super::helpers::{create_test_consumer, create_test_invoice};

fn make_payment(
    invoice_id: &InvoiceId,
    amount: u64,
    tx_hash: B256,
    is_native: bool,
    token_address: Option<Address>,
) -> MonitorEvent {
    MonitorEvent::PaymentDetected(PaymentDetected {
        chain_id: 1,
        invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: Address::ZERO,
        amount: U256::from(amount),
        tx_hash,
        block_number: 12_345_678,
        block_hash: B256::ZERO,
        log_index: None,
        is_native,
        token_address,
        from_address: Address::repeat_byte(0xab),
        confirmations: 1,
        required_confirmations: 12,
        detected_at: Utc::now(),
    })
}

/// An event that can never apply must halt too, not be skipped: a skip
/// would commit the cursor past a payment nobody credited.
#[tokio::test]
async fn a_permanently_bad_event_halts_without_committing_the_cursor() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let invoice = InvoiceId::new();
    create_test_invoice(&ds, &invoice, StoreId::new()).await;

    // Malformed: `is_native: false` with no token address is rejected by
    // `handle_payment_detected` before it touches the database at all, a
    // deterministic and DB-free way to make `handle_event` fail.
    bridge
        .publish(&make_payment(
            &invoice,
            1,
            B256::from([9u8; 32]),
            false,
            None,
        ))
        .await
        .unwrap();

    let failures: Arc<Mutex<Vec<(u64, i64)>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = failures.clone();
    let consumer = create_test_consumer(ds.clone(), bridge.clone()).with_apply_failure_hook(
        Arc::new(move |chain_id, seq| {
            recorded.lock().unwrap().push((chain_id, seq));
        }),
    );
    let task = tokio::spawn(consumer.run());

    tokio::time::timeout(Duration::from_secs(2), async {
        while failures.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("halt hook was never called");
    let _ = tokio::time::timeout(Duration::from_secs(1), task).await;

    let cursors = ChainCursorReader::chain_cursors(&*ds, ADAPTER_ID)
        .await
        .unwrap();
    assert_eq!(cursors.get(&1), None);
}

/// A possibly-transient failure (the database) must halt, and the cursor
/// must not move past the event that failed.
#[tokio::test]
async fn a_transient_apply_failure_halts_without_committing_the_cursor() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let store_id = StoreId::new();

    let invoice = InvoiceId::new();
    create_test_invoice(&ds, &invoice, store_id).await;
    for (amount, byte) in [(1u64, 1u8), (2, 2)] {
        bridge
            .publish(&make_payment(
                &invoice,
                amount,
                B256::from([byte; 32]),
                true,
                None,
            ))
            .await
            .unwrap();
    }
    ds.fail_payment_writes();

    let failures: Arc<Mutex<Vec<(u64, i64)>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = failures.clone();
    let consumer = create_test_consumer(ds.clone(), bridge.clone()).with_apply_failure_hook(
        Arc::new(move |chain_id, seq| {
            recorded.lock().unwrap().push((chain_id, seq));
        }),
    );
    let task = tokio::spawn(consumer.run());

    tokio::time::timeout(Duration::from_secs(2), async {
        while failures.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("apply failure hook was never called");
    let _ = tokio::time::timeout(Duration::from_secs(1), task).await;

    assert_eq!(*failures.lock().unwrap(), vec![(1, 0)]);
    let cursors = ChainCursorReader::chain_cursors(&*ds, ADAPTER_ID)
        .await
        .unwrap();
    assert_eq!(cursors.get(&1), None);
}

/// A `handle_event` success followed by a failed `commit_chain_cursor` write
/// must halt exactly like a `handle_event` failure does above - the effect
/// applied, but nothing durable points at that fact, so a later envelope on
/// the same chain must not be allowed to commit a cursor past it.
#[tokio::test]
async fn a_failed_cursor_commit_stops_a_later_envelope_on_the_same_chain_from_committing_past_it() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let store_id = StoreId::new();

    // seq 0: applies cleanly, but its cursor commit is made to fail below.
    let good_invoice = InvoiceId::new();
    create_test_invoice(&ds, &good_invoice, store_id).await;
    bridge
        .publish(&make_payment(
            &good_invoice,
            1,
            B256::from([1u8; 32]),
            true,
            None,
        ))
        .await
        .unwrap();

    // seq 1, same chain. With the bug this test guards against, the
    // consumer would keep going after the swallowed commit failure and
    // apply this one anyway.
    let later_invoice = InvoiceId::new();
    create_test_invoice(&ds, &later_invoice, store_id).await;
    bridge
        .publish(&make_payment(
            &later_invoice,
            2,
            B256::from([2u8; 32]),
            true,
            None,
        ))
        .await
        .unwrap();

    ds.set_fail_commit_chain_cursor(true);

    let failures: Arc<Mutex<Vec<(u64, i64)>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = failures.clone();
    let consumer = create_test_consumer(ds.clone(), bridge.clone()).with_apply_failure_hook(
        Arc::new(move |chain_id, seq| {
            recorded.lock().unwrap().push((chain_id, seq));
        }),
    );

    let task = tokio::spawn(consumer.run());

    tokio::time::timeout(Duration::from_secs(2), async {
        while failures.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("apply failure hook was never called");
    let _ = tokio::time::timeout(Duration::from_secs(1), task).await;

    assert_eq!(*failures.lock().unwrap(), vec![(1, 0)]);

    // The envelope after the one whose commit failed must never have been
    // applied.
    let later_payments = PaymentReader::get_for_invoice(&*ds, &later_invoice)
        .await
        .unwrap();
    assert!(
        later_payments.is_empty(),
        "an envelope after a failed cursor commit was applied - the consumer kept going \
         instead of halting"
    );

    // No cursor for the chain was ever durably committed.
    let cursors = ChainCursorReader::chain_cursors(&*ds, ADAPTER_ID)
        .await
        .unwrap();
    assert_eq!(cursors.get(&1), None);
}

/// The operator's way past a poison event: naming its `(chain_id, seq)` lets
/// the consumer commit past it and carry on, while the same event unnamed
/// still halts (previous tests). Only the named envelope is skipped.
#[tokio::test]
async fn a_named_poison_event_is_skipped_and_the_next_one_applies() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let invoice = InvoiceId::new();
    create_test_invoice(&ds, &invoice, StoreId::new()).await;

    // seq 0: malformed, can never apply. seq 1: valid.
    bridge
        .publish(&make_payment(
            &invoice,
            1,
            B256::from([9u8; 32]),
            false,
            None,
        ))
        .await
        .unwrap();
    bridge
        .publish(&make_payment(
            &invoice,
            2,
            B256::from([2u8; 32]),
            true,
            None,
        ))
        .await
        .unwrap();

    let failures: Arc<Mutex<Vec<(u64, i64)>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = failures.clone();
    let consumer = create_test_consumer(ds.clone(), bridge.clone())
        .with_skipped_events([(1, 0)])
        .with_apply_failure_hook(Arc::new(move |chain_id, seq| {
            recorded.lock().unwrap().push((chain_id, seq));
        }));
    let task = tokio::spawn(consumer.run());

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let cursors = ChainCursorReader::chain_cursors(&*ds, ADAPTER_ID)
                .await
                .unwrap();
            if cursors.get(&1).is_some_and(|c| c.seq == 1) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("cursor never advanced past the skipped event");
    task.abort();

    assert!(failures.lock().unwrap().is_empty());
    let payments = PaymentReader::get_for_invoice(&*ds, &invoice)
        .await
        .unwrap();
    assert_eq!(payments.len(), 1, "only the valid event is applied");
}
