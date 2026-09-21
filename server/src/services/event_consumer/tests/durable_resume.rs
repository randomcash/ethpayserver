#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Proves the hole this whole mechanism exists to close: a payment that
//! confirms while the consumer is down must still be credited once it comes
//! back, rather than being gone the way plain pub/sub would drop it.
//!
//! Against the old `EventConsumer::run`, this scenario was not survivable
//! at all - `MemoryBridge::subscribe` (mirroring `RedisBridge`'s
//! `PUBLISH`/`SUBSCRIBE`) delivered only to a receiver that was already
//! listening, with no backlog and no cursor to resume from. There was
//! nothing for a restarted consumer to ask for. This test exercises the
//! durable outbox and `chain_cursors` this commit adds in their place.

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
    .expect("payment was never credited after the consumer resumed");
}

#[tokio::test]
async fn a_payment_published_while_the_consumer_is_down_is_still_credited_on_restart() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let store_id = StoreId::new();

    // Run a first consumer long enough to apply one event and commit a real
    // cursor - the interesting case is a restart that has something to
    // resume from, not a cold boot with an empty `chain_cursors`.
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
    // kill below; both are plain in-memory writes on the same task, but
    // this removes any doubt about which one the abort races.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Kill it - no graceful shutdown, the same as a crash or a redeploy.
    task1.abort();
    let _ = task1.await;

    // Pay an invoice while nothing is subscribed to the outbox at all.
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

    // Restart, resuming from the committed cursor rather than "now".
    let consumer2 = create_test_consumer(ds.clone(), bridge.clone());
    let task2 = tokio::spawn(consumer2.run());
    wait_for_payment(&ds, &invoice_id).await;
    task2.abort();
    let _ = task2.await;

    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert_eq!(payments.len(), 1);
    assert_eq!(payments[0].amount, "500000000000000000");
    assert!(!payments[0].reorged);
}
