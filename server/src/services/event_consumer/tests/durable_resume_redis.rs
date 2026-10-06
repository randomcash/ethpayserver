#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The kill-the-consumer, pay, restart scenario against a real
//! `RedisBridge`, so the consumer is proven to compose with the real epoch
//! source and the outbox's stream framing rather than only with the
//! in-process `MemoryBridge`.
//!
//! Needs `TEST_REDIS_URL`, e.g. `redis://127.0.0.1:6379`; CI's `test` job
//! sets it and runs ignored tests.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use data_service::InMemoryDataService;
use evm::monitor::bridge::{EventBridge, RedisBridge};
use evm::monitor::events::{MonitorEvent, PaymentDetected};
use evm::{Address, B256, U256};
use types::{InvoiceId, PaymentReader, StoreId};

use super::helpers::{create_test_consumer_with_bridge, create_test_invoice};

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
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let payments = PaymentReader::get_for_invoice(ds, invoice_id)
                .await
                .unwrap();
            if !payments.is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("payment was never credited after the consumer resumed");
}

#[tokio::test]
#[ignore = "requires a local Redis instance; set TEST_REDIS_URL, e.g. redis://127.0.0.1:6379"]
async fn a_payment_published_while_the_consumer_is_down_is_credited_on_restart_via_redis() {
    let url = std::env::var("TEST_REDIS_URL").expect("TEST_REDIS_URL required");
    // Fresh keys per run so reruns never share an outbox, epoch or seq.
    let suffix = uuid::Uuid::new_v4();
    let bridge: Arc<dyn EventBridge> = Arc::new(
        RedisBridge::new(
            &url,
            &format!("test:consumer_resume:{suffix}:events"),
            &format!("test:consumer_resume:{suffix}:commands"),
        )
        .await
        .expect("connect to TEST_REDIS_URL"),
    );
    let ds = Arc::new(InMemoryDataService::new());
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

    let task1 = tokio::spawn(create_test_consumer_with_bridge(ds.clone(), bridge.clone()).run());
    wait_for_payment(&ds, &warm_invoice_id).await;
    // Let the cursor commit that follows the apply land before the kill.
    tokio::time::sleep(Duration::from_millis(100)).await;
    task1.abort();
    let _ = task1.await;

    // Pay while nothing is subscribed: plain PUBLISH would drop this.
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

    let task2 = tokio::spawn(create_test_consumer_with_bridge(ds.clone(), bridge.clone()).run());
    wait_for_payment(&ds, &invoice_id).await;
    task2.abort();
    let _ = task2.await;

    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert_eq!(payments.len(), 1);
    assert_eq!(payments[0].amount, "500000000000000000");
}
