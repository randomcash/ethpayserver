#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The low-water mark in `reconcile_cursors` resumes every chain from the
//! *slowest* chain's committed position, so a chain further ahead re-sees
//! entries it already applied. `apply_envelope`'s dedup check exists to
//! make that safe. Every other consumer test uses a single chain, where the
//! low-water mark trivially equals that chain's own cursor and the dedup
//! branch is never taken - this uses two chains at different committed
//! positions so it actually is.

use std::sync::Arc;
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
async fn a_chain_further_ahead_than_the_low_water_mark_does_not_double_credit_on_resume() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let store_id = StoreId::new();

    // Chain 1 gets two payments (it will be the one further ahead); chain 2
    // gets one (it will be the low-water mark). All three share the one
    // outbox, so chain 1's committed cursor ends up ahead of chain 2's in
    // the shared `seq` numbering.
    create_and_publish(&ds, &bridge, store_id, 1, 1, 1).await;
    let chain1_second = create_and_publish(&ds, &bridge, store_id, 1, 2, 2).await;
    let chain2_first = create_and_publish(&ds, &bridge, store_id, 2, 3, 3).await;

    let consumer1 = create_test_consumer(ds.clone(), bridge.clone());
    let task1 = tokio::spawn(consumer1.run());
    wait_for_payment(&ds, &chain1_second).await;
    wait_for_payment(&ds, &chain2_first).await;
    // Let both cursor commits land: chain 1 at the shared outbox's newest
    // seq, chain 2 at an older one - the low-water mark on restart is
    // chain 2's, strictly behind chain 1's own committed position.
    tokio::time::sleep(Duration::from_millis(50)).await;
    task1.abort();
    let _ = task1.await;

    // Restart: resuming from the low-water mark re-delivers chain 2's
    // already-applied entry (and possibly chain 1's) to the new consumer.
    let chain2_second = create_and_publish(&ds, &bridge, store_id, 2, 4, 4).await;

    let consumer2 = create_test_consumer(ds.clone(), bridge.clone());
    let task2 = tokio::spawn(consumer2.run());
    wait_for_payment(&ds, &chain2_second).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    task2.abort();
    let _ = task2.await;

    let chain1_payments = PaymentReader::get_for_invoice(&*ds, &chain1_second)
        .await
        .unwrap();
    assert_eq!(
        chain1_payments.len(),
        1,
        "chain 1's already-applied entry must not be re-credited when re-seen via the shared \
         low-water mark"
    );

    let chain2_first_payments = PaymentReader::get_for_invoice(&*ds, &chain2_first)
        .await
        .unwrap();
    assert_eq!(
        chain2_first_payments.len(),
        1,
        "chain 2's own already-applied entry must not be double-credited either"
    );

    let chain2_second_payments = PaymentReader::get_for_invoice(&*ds, &chain2_second)
        .await
        .unwrap();
    assert_eq!(chain2_second_payments.len(), 1);
    assert_eq!(chain2_second_payments[0].amount, "4");
}
