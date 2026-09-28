#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Besides the epoch check in `reconcile_cursors`, the outbox can report a
//! stored position as trimmed - `Err(EvmError::EventStreamOutOfRange)` from `subscribe_from`
//! itself - even when the epoch it was committed under still matches, since
//! retention trims independently of the epoch key. The consumer must not
//! resume from the oldest retained entry then: the events in between are
//! gone, and only a halt (which pages someone) keeps that from being silent.
//! This test gives `MemoryBridge` a real retention cap to drive that branch.

use std::sync::Arc;
use std::sync::Mutex;
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
    .expect("payment was never credited after the consumer resumed")
}

#[tokio::test]
async fn a_resume_position_trimmed_out_from_under_the_consumer_halts_instead_of_skipping_the_gap() {
    let ds = Arc::new(InMemoryDataService::new());
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
    // Let the cursor commit that follows the apply land before the kill.
    tokio::time::sleep(Duration::from_millis(50)).await;
    task1.abort();
    let _ = task1.await;

    // Push the committed cursor out of the retention window while nothing is
    // subscribed; the epoch never changes here, only retention does.
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

    let reasons: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = reasons.clone();
    let consumer2 = create_test_consumer(ds.clone(), bridge.clone()).with_resume_failure_hook(
        Arc::new(move |reason| recorded.lock().unwrap().push(reason.to_string())),
    );
    let task2 = tokio::spawn(consumer2.run());
    tokio::time::timeout(Duration::from_secs(2), async {
        while reasons.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("an out-of-range resume never invoked the failure hook");
    let _ = tokio::time::timeout(Duration::from_secs(1), task2).await;

    assert!(reasons.lock().unwrap()[0].contains("out of range"));
    // Nothing past the gap may have been applied on the way out.
    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert!(payments.is_empty(), "the consumer resumed past the gap");
}
