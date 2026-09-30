#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Against an unbounded `MemoryBridge`, `subscribe_from(None)` and
//! `subscribe_from(Some(the_real_cursor))` return the same entries - nothing
//! has been trimmed - and `apply_envelope`'s dedup check is driven by the
//! `cursors` map loaded independently from `data_service.chain_cursors()`,
//! not by whatever was actually passed to `subscribe_from`. So a payment
//! ending up credited after a restart (as `durable_resume.rs` checks)
//! proves resume works end-to-end, but does not by itself prove `run`
//! resumed from the *persisted* cursor rather than from scratch and let
//! dedup paper over the difference - a future change that silently
//! hardcoded `subscribe_from(None)` on the restart path would drop the one
//! behavior this whole mechanism exists to add, and every outcome-based
//! test in this suite would still pass.
//!
//! This asserts the actual argument instead, via a bridge that records it.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use data_service::InMemoryDataService;
use evm::monitor::bridge::{EventBridge, EventCursor, MemoryBridge};
use evm::monitor::events::{MonitorEvent, PaymentDetected};
use evm::{Address, B256, U256};
use types::{InvoiceId, PaymentReader, StoreId};

use super::helpers::{
    RecordingBridge, create_test_consumer, create_test_consumer_with_bridge, create_test_invoice,
};

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
    .expect("payment was never credited after the consumer resumed");
}

#[tokio::test]
async fn a_restart_subscribes_with_the_persisted_cursor_not_from_scratch() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let store_id = StoreId::new();

    // Warm the outbox and let a first consumer apply and commit a real
    // cursor, the same setup `durable_resume.rs` uses.
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
    tokio::time::sleep(Duration::from_millis(50)).await;
    task1.abort();
    let _ = task1.await;

    let committed = data_service::ChainCursorReader::chain_cursors(&*ds, "evmmonitor")
        .await
        .unwrap();
    let persisted_cursor = *committed
        .get(&1)
        .expect("first run should have committed a cursor for chain 1");

    // Pay again while nothing is subscribed, then restart through a bridge
    // that records what `subscribe_from` was actually called with.
    let invoice_id = InvoiceId::new();
    create_test_invoice(&ds, &invoice_id, store_id).await;
    bridge
        .publish(&make_payment_detected(
            &invoice_id,
            500_000_000_000_000_000,
            B256::from([2u8; 32]),
        ))
        .await
        .unwrap();

    let recording_bridge = Arc::new(RecordingBridge::new(bridge.clone()));
    let consumer2 = create_test_consumer_with_bridge(ds.clone(), recording_bridge.clone());
    let task2 = tokio::spawn(consumer2.run());
    wait_for_payment(&ds, &invoice_id).await;
    task2.abort();
    let _ = task2.await;

    let calls = recording_bridge.subscribe_from_calls();
    assert_eq!(
        calls,
        vec![Some(EventCursor {
            epoch: persisted_cursor.epoch,
            seq: persisted_cursor.seq,
            // `reconcile_cursors` always returns 0 here regardless of what
            // was persisted - nothing reads `block_height` back to decide
            // where to resume, only `seq`/`epoch` - so 0 is the correct
            // expectation, not a stand-in for the persisted value.
            block_height: 0,
        })],
        "restart must resume from the cursor chain_cursors persisted, not from scratch - a \
         hardcoded `subscribe_from(None)` here would still pass every outcome-based assertion \
         in this suite thanks to idempotent dedup"
    );
}
