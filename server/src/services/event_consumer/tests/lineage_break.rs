#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The other half of the durable-resume guarantee: a cursor is only safe to
//! resume from while it names a lineage the outbox still vouches for.
//!
//! Before `EventBridge::bump_epoch` existed, nothing in either bridge ever
//! changed an outbox's epoch after its first init, so `reconcile_cursors`'s
//! mismatch branch - the code that re-arms `watch_retry` rather than
//! silently resuming "from now" - had no way to be reached except by
//! fabricating a mismatched epoch by hand. This exercises it through the
//! real `EventConsumer::run` path: a lineage break between two consumer
//! runs, not a literal passed straight to a private method.

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
    .expect("payment was never credited after the lineage break")
}

#[tokio::test]
async fn an_outbox_lineage_break_re_arms_watch_retry_and_still_credits_the_next_payment() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let store_id = StoreId::new();

    // Run a first consumer long enough to commit a real cursor under the
    // outbox's original epoch.
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

    assert_eq!(
        ds.watch_reset_calls(),
        0,
        "nothing should have re-armed watch_retry yet"
    );

    // The outbox itself loses continuity - a full data loss on whatever
    // backs it in production, not a process restart. `seq` numbers from
    // before this point name a lineage that no longer exists.
    bridge.bump_epoch().await.unwrap();

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

    // Restart against the broken lineage: the stored cursor's epoch no
    // longer matches the outbox's, so this must not resume from it as if
    // nothing happened.
    let consumer2 = create_test_consumer(ds.clone(), bridge.clone());
    let task2 = tokio::spawn(consumer2.run());
    wait_for_payment(&ds, &invoice_id).await;
    task2.abort();
    let _ = task2.await;

    assert!(
        ds.watch_reset_calls() > 0,
        "a lineage break must re-arm watch_retry rather than resume silently"
    );

    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert_eq!(payments.len(), 1);
    assert_eq!(payments[0].amount, "500000000000000000");
}
