#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The gap `apply_envelope`'s own doc comment calls out: `handle_event` can
//! succeed and then the process can die (or `commit_chain_cursor` can fail)
//! before that success is durable. On restart, nothing in memory survived -
//! a fresh consumer loads cursors from scratch, sees no committed position
//! for this chain, and the outbox redelivers the same envelope with none of
//! `apply_envelope`'s own low-water-mark guard in play at all, unlike
//! `multi_chain_resume.rs`, where that in-memory guard is exactly what stops
//! the redelivered entry from reapplying. This is the one path a redelivery
//! reaches `handle_event` for real, and the payment upsert alone being
//! idempotent is not enough: `handle_payment_detected` mints a fresh
//! `payment.id` on every call, so whatever dedups the webhook obligation it
//! writes alongside the row has to do it without that id ever repeating.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use data_service::{ChainCursorReader, InMemoryDataService, WebhookOutboxReader};
use evm::monitor::bridge::{EventBridge, MemoryBridge};
use evm::monitor::events::{MonitorEvent, PaymentDetected};
use evm::{Address, B256, U256};
use types::{InvoiceId, PaymentReader, StoreId};

use super::super::ADAPTER_ID;
use super::helpers::{create_test_consumer, create_test_invoice};

fn make_payment(invoice_id: &InvoiceId, amount: u64, tx_hash: B256) -> MonitorEvent {
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

#[tokio::test]
async fn a_restart_after_a_successful_apply_but_a_failed_cursor_commit_does_not_double_queue_the_webhook()
 {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let store_id = StoreId::new();

    let invoice_id = InvoiceId::new();
    create_test_invoice(&ds, &invoice_id, store_id).await;
    bridge
        .publish(&make_payment(&invoice_id, 1, B256::from([1u8; 32])))
        .await
        .unwrap();

    // Force the durable cursor write to fail after `handle_event` has
    // already applied the payment - the exact window `apply_envelope`'s doc
    // comment says is fine only because the apply is idempotent. This
    // proves the *other* effect of that same apply, the webhook obligation,
    // is idempotent too.
    ds.set_fail_commit_chain_cursor(true);

    let failures: Arc<Mutex<Vec<(u64, i64)>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = failures.clone();
    let consumer1 = create_test_consumer(ds.clone(), bridge.clone()).with_apply_failure_hook(
        Arc::new(move |chain_id, seq| {
            recorded.lock().unwrap().push((chain_id, seq));
        }),
    );

    let task1 = tokio::spawn(consumer1.run());
    tokio::time::timeout(Duration::from_secs(2), async {
        while failures.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("apply failure hook was never called");
    let _ = tokio::time::timeout(Duration::from_secs(1), task1).await;

    // The apply itself went through: the payment is on file and its
    // obligation was recorded in the same write.
    let payments_after_crash = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert_eq!(payments_after_crash.len(), 1);

    // But nothing durable points at that fact - the crash this test
    // simulates landed after the effect but before the cursor commit that
    // would have recorded it.
    let cursors_after_crash = ChainCursorReader::chain_cursors(&*ds, ADAPTER_ID)
        .await
        .unwrap();
    assert_eq!(cursors_after_crash.get(&1), None);

    // Restart: a brand new consumer, its own empty in-memory `cursors` map -
    // nothing survives from consumer1. It resumes from scratch, the bridge
    // redelivers the same envelope, and this time the cursor commit is
    // allowed to succeed.
    ds.set_fail_commit_chain_cursor(false);
    let consumer2 = create_test_consumer(ds.clone(), bridge.clone());
    let task2 = tokio::spawn(consumer2.run());

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let cursors = ChainCursorReader::chain_cursors(&*ds, ADAPTER_ID)
                .await
                .unwrap();
            if cursors.get(&1).map(|c| c.seq) == Some(0) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the redelivered envelope never committed a durable cursor");
    task2.abort();
    let _ = task2.await;

    // The redelivered envelope must not have credited the payment twice.
    let payments_after_resume = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert_eq!(
        payments_after_resume.len(),
        1,
        "the payment upsert is keyed on (chain_id, tx_hash, tx_index), so this passing does \
         not by itself prove the redelivery was handled - see the obligation count below"
    );

    // The actual claim under test: `handle_payment_detected` ran a second
    // time (a fresh `payment.id`, a fresh call into
    // `upsert_with_tx_index_and_obligation`), and the webhook obligation it
    // writes alongside the row must still be exactly one, not two.
    let obligations = ds.claim_undispatched_obligations(100, 300).await.unwrap();
    assert_eq!(
        obligations.len(),
        1,
        "a redelivery after a crashed cursor commit must not queue a second webhook \
         obligation for the same payment"
    );
}
