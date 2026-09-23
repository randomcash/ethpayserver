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

#[tokio::test]
async fn a_failed_apply_stops_a_later_envelope_on_the_same_chain_from_committing_past_it() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let store_id = StoreId::new();

    // seq 0: applies cleanly.
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

    // seq 1: malformed - `is_native: false` with no token address is
    // rejected by `handle_payment_detected` before it touches the
    // database at all, a deterministic and DB-free way to make
    // `handle_event` fail.
    bridge
        .publish(&make_payment(
            &good_invoice,
            1,
            B256::from([2u8; 32]),
            false,
            None,
        ))
        .await
        .unwrap();

    // seq 2: well-formed, on the same chain. With the bug this test
    // guards against, the consumer would skip past the failed seq 1 and
    // apply this one anyway, silently losing seq 1 for good.
    let later_invoice = InvoiceId::new();
    create_test_invoice(&ds, &later_invoice, store_id).await;
    bridge
        .publish(&make_payment(
            &later_invoice,
            2,
            B256::from([3u8; 32]),
            true,
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
    .expect("apply failure hook was never called");
    // The hook doesn't exit, so `run`'s own loop is what has to stop; give
    // it a moment to actually return rather than asserting mid-flight.
    let _ = tokio::time::timeout(Duration::from_secs(1), task).await;

    assert_eq!(*failures.lock().unwrap(), vec![(1, 1)]);

    // The envelope after the failed one must never have been applied.
    let later_payments = PaymentReader::get_for_invoice(&*ds, &later_invoice)
        .await
        .unwrap();
    assert!(
        later_payments.is_empty(),
        "an envelope after a failed one was applied - the consumer skipped past the failure \
         instead of halting"
    );

    // The persisted cursor must sit at the last successful apply (seq 0),
    // not skip forward past the failed seq 1.
    let cursors = ChainCursorReader::chain_cursors(&*ds, ADAPTER_ID)
        .await
        .unwrap();
    assert_eq!(cursors.get(&1).map(|c| c.seq), Some(0));
}
