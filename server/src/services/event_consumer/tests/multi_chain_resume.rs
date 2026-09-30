#![allow(clippy::unwrap_used, clippy::expect_used)]
//! `seq` is one counter for the whole outbox but cursors are per chain, and
//! a chain's cursor only moves when an event for that chain arrives. The
//! resume point must therefore be the highest committed `seq`, not the
//! lowest: an idle chain's old cursor would otherwise pin the resume point
//! behind the retention window and make ordinary traffic on other chains
//! look like a lost gap (`OUT_OF_RANGE`) on every restart.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use data_service::InMemoryDataService;
use evm::monitor::bridge::{EventBridge, MemoryBridge};
use evm::monitor::events::{MonitorEvent, PaymentDetected};
use evm::{Address, B256, U256};
use types::{InvoiceId, PaymentReader, StoreId};

use super::helpers::{create_test_consumer, create_test_invoice};

fn make_payment_detected(
    chain_id: u64,
    invoice_id: &InvoiceId,
    amount: u64,
    tx_hash: B256,
) -> MonitorEvent {
    MonitorEvent::PaymentDetected(PaymentDetected {
        chain_id,
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

/// Create an invoice and publish a payment for it on `chain_id`, in one
/// call. Every case below does this and nothing else with the
/// invoice/publish pair, so spelling it out each time is what pushed the
/// test past clippy's line limit.
async fn create_and_publish(
    ds: &InMemoryDataService,
    bridge: &MemoryBridge,
    store_id: StoreId,
    chain_id: u64,
    amount: u64,
    tx_byte: u8,
) -> InvoiceId {
    let invoice_id = InvoiceId::new();
    create_test_invoice(ds, &invoice_id, store_id).await;
    bridge
        .publish(&make_payment_detected(
            chain_id,
            &invoice_id,
            amount,
            B256::from([tx_byte; 32]),
        ))
        .await
        .unwrap();
    invoice_id
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
    .expect("payment was never credited")
}

#[tokio::test]
async fn an_idle_chain_does_not_pin_the_resume_point_behind_retention() {
    let ds = Arc::new(InMemoryDataService::new());
    // Retains only 3 entries, so the idle chain's seq is trimmed away below.
    let bridge = Arc::new(MemoryBridge::with_max_retained(3));
    let store_id = StoreId::new();

    let idle = create_and_publish(&ds, &bridge, store_id, 2, 3, 3).await;
    let task1 = tokio::spawn(create_test_consumer(ds.clone(), bridge.clone()).run());
    wait_for_payment(&ds, &idle).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Chain 1 then moves on far enough to trim chain 2's committed seq out
    // of the outbox, while chain 2 sees nothing further.
    // One at a time: a burst would trim entries the live consumer has not
    // read yet, which is a different (and correctly fatal) gap.
    for i in 0..5u8 {
        let id = create_and_publish(&ds, &bridge, store_id, 1, 1 + u64::from(i), 10 + i).await;
        wait_for_payment(&ds, &id).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    task1.abort();
    let _ = task1.await;

    let after = create_and_publish(&ds, &bridge, store_id, 1, 9, 99).await;
    let reasons: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = reasons.clone();
    let consumer2 = create_test_consumer(ds.clone(), bridge.clone()).with_resume_failure_hook(
        Arc::new(move |reason| recorded.lock().unwrap().push(reason.to_string())),
    );
    let task2 = tokio::spawn(consumer2.run());
    wait_for_payment(&ds, &after).await;
    task2.abort();
    let _ = task2.await;

    assert!(
        reasons.lock().unwrap().is_empty(),
        "nothing was missed, so the restart must not fail: {:?}",
        reasons.lock().unwrap()
    );
}
