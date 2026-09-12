#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::Utc;
use data_service::InMemoryDataService;
use evm::B256;
use evm::monitor::bridge::MemoryBridge;
use evm::monitor::events::ReorgDetected;
use std::sync::Arc;
use types::ChainId;
use types::{
    InvoiceData, InvoiceId, InvoiceReader, InvoiceStatus, InvoiceWriter, PaymentData,
    PaymentReader, PaymentWriter, StoreId,
};
use uuid::Uuid;

use crate::services::webhook::WebhookEventType;

use super::helpers::{
    RecordingWebhookSink, create_test_consumer, create_test_consumer_with_webhook,
};

#[tokio::test]
async fn test_handle_reorg_detected() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds.clone(), bridge.clone());

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();

    // Create invoice in processing state
    let invoice = InvoiceData {
        id: invoice_id.clone(),
        store_id,
        currency: "ETH".to_string(),
        status: InvoiceStatus::Processing,
        amount: "1000000000000000000".to_string(),
        amount_received: "500000000000000000".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        customer_email: None,
        extra: None,
    };
    InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

    // Create a payment at block 100
    let payment = PaymentData {
        id: Uuid::new_v4(),
        invoice_id: invoice_id.clone(),
        payment_option_id: None,
        chain_id: ChainId::parse("eip155:1").unwrap(),
        asset_type: types::AssetType::Native,
        amount: "500000000000000000".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        tx_hash: "0xabc123".to_string(),
        block_number: Some(100),
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: None,
        reorged: false,
        extra: None,
        credited_amount: Some("0.5".to_string()),
        rate_used: None,
        rate_applied_at: None,
    };
    PaymentWriter::upsert(&*ds, &payment).await.unwrap();

    // Create ReorgDetected event at block 99 (affecting block 100)
    let event = ReorgDetected {
        chain_id: 1,
        fork_block: 99,
        old_hash: B256::ZERO,
        new_hash: B256::repeat_byte(0x01),
        depth: 2,
        affected_invoices: vec![uuid::Uuid::parse_str(invoice_id.as_str()).unwrap()],
        detected_at: Utc::now(),
    };

    // Handle the event
    consumer.handle_reorg_detected(event).await.unwrap();

    // Verify payment was marked as reorged
    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert!(payments[0].reorged);

    // Verify invoice was reverted to pending (no valid payments)
    let invoice = InvoiceReader::get(&*ds, &invoice_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(invoice.status, InvoiceStatus::Pending);
}

#[tokio::test]
async fn test_handle_reorg_with_remaining_valid_payments() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds.clone(), bridge.clone());

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();

    // Create invoice in processing state
    let invoice = fully_paid_invoice(&invoice_id, store_id);
    InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

    // Create first payment at block 50 (will NOT be reorged)
    let payment1 = PaymentData {
        id: Uuid::new_v4(),
        invoice_id: invoice_id.clone(),
        payment_option_id: None,
        chain_id: ChainId::parse("eip155:1").unwrap(),
        asset_type: types::AssetType::Native,
        amount: "500000000000000000".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        tx_hash: "0xearly".to_string(),
        block_number: Some(50),
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: None,
        reorged: false,
        extra: None,
        credited_amount: Some("0.5".to_string()),
        rate_used: None,
        rate_applied_at: None,
    };
    PaymentWriter::upsert(&*ds, &payment1).await.unwrap();

    // Create second payment at block 100 (will be reorged)
    let payment2 = PaymentData {
        id: Uuid::new_v4(),
        invoice_id: invoice_id.clone(),
        payment_option_id: None,
        chain_id: ChainId::parse("eip155:1").unwrap(),
        asset_type: types::AssetType::Native,
        amount: "500000000000000000".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        tx_hash: "0xlate".to_string(),
        block_number: Some(100),
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: None,
        reorged: false,
        extra: None,
        credited_amount: Some("0.5".to_string()),
        rate_used: None,
        rate_applied_at: None,
    };
    PaymentWriter::upsert(&*ds, &payment2).await.unwrap();

    // Create ReorgDetected event at block 99
    let event = ReorgDetected {
        chain_id: 1,
        fork_block: 99,
        old_hash: B256::ZERO,
        new_hash: B256::repeat_byte(0x01),
        depth: 2,
        affected_invoices: vec![uuid::Uuid::parse_str(invoice_id.as_str()).unwrap()],
        detected_at: Utc::now(),
    };

    // Handle the event
    consumer.handle_reorg_detected(event).await.unwrap();

    // Verify only one payment was reorged
    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    let reorged_count = payments.iter().filter(|p| p.reorged).count();
    assert_eq!(reorged_count, 1);

    // Invoice should be processing (still has valid payments)
    let invoice = InvoiceReader::get(&*ds, &invoice_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(invoice.status, InvoiceStatus::Processing);
}

/// A Processing invoice with its full amount already received.
///
/// Extracted from the reorg test rather than inlined: the literal is long
/// enough that adding a field to InvoiceData pushed the test over clippy's
/// 80-line limit, which is a signal the setup belonged in a helper anyway.
/// `helpers::create_test_invoice` cannot be reused here - it builds a Pending
/// invoice with nothing received, which is the opposite of what a reorg test
/// needs.
fn fully_paid_invoice(invoice_id: &InvoiceId, store_id: StoreId) -> InvoiceData {
    InvoiceData {
        id: invoice_id.clone(),
        store_id,
        currency: "ETH".to_string(),
        status: InvoiceStatus::Processing,
        amount: "1000000000000000000".to_string(),
        amount_received: "1000000000000000000".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        customer_email: None,
        extra: None,
    }
}

/// The point of the retraction event: a subscriber that was told about a
/// payment is told when that payment stops existing. Before this event the
/// invoice silently moved backwards and nothing was sent.
#[tokio::test]
async fn test_reorg_emits_retraction_webhook() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let sink = Arc::new(RecordingWebhookSink::new());
    let consumer = create_test_consumer_with_webhook(ds.clone(), bridge.clone(), sink.clone());

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();
    ds.set_webhook(store_id.0, "https://example.com/hook", "secret");

    let invoice = fully_paid_invoice(&invoice_id, store_id);
    InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

    let payment = reorgable_payment(&invoice_id, "0xdoomed", 100);
    PaymentWriter::upsert(&*ds, &payment).await.unwrap();

    consumer
        .handle_reorg_detected(reorg_at(&invoice_id, 99))
        .await
        .unwrap();

    let jobs = sink.jobs();
    assert_eq!(jobs.len(), 1, "a reorg that retracts a payment emits once");

    let payload = &jobs[0].payload;
    assert_eq!(payload.event_type, WebhookEventType::PaymentReorged);
    assert_eq!(payload.invoice_id, invoice_id.as_str());
    assert_eq!(payload.store_id, store_id.0);

    // The status the invoice reverted to, not the one the subscriber already
    // knows is wrong.
    assert_eq!(payload.status, "pending");

    let retracted = payload
        .retracted_payments
        .as_ref()
        .expect("a retraction names the payments it retracts");
    assert_eq!(retracted.len(), 1);
    assert_eq!(retracted[0].tx_hash, "0xdoomed");
    assert!(!retracted[0].confirmed);

    // Distinct from a resent original: type differs, and so does the key a
    // subscriber dedupes on.
    assert!(payload.payment.is_none());
    assert!(!payload.idempotency_key.is_empty());
}

/// A reorg that touches nothing must not tell a subscriber to undo anything.
#[tokio::test]
async fn test_reorg_without_affected_payments_emits_nothing() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let sink = Arc::new(RecordingWebhookSink::new());
    let consumer = create_test_consumer_with_webhook(ds.clone(), bridge.clone(), sink.clone());

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();
    ds.set_webhook(store_id.0, "https://example.com/hook", "secret");

    let invoice = fully_paid_invoice(&invoice_id, store_id);
    InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

    // Payment is far below the fork block, so the reorg cannot have undone it.
    let payment = reorgable_payment(&invoice_id, "0xsafe", 10);
    PaymentWriter::upsert(&*ds, &payment).await.unwrap();

    consumer
        .handle_reorg_detected(reorg_at(&invoice_id, 99))
        .await
        .unwrap();

    assert!(sink.jobs().is_empty());
}

/// Re-running the same reorg produces the same idempotency key, so a
/// subscriber that already undid the credit drops the repeat.
#[tokio::test]
async fn test_replayed_reorg_carries_the_same_idempotency_key() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let sink = Arc::new(RecordingWebhookSink::new());
    let consumer = create_test_consumer_with_webhook(ds.clone(), bridge.clone(), sink.clone());

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();
    ds.set_webhook(store_id.0, "https://example.com/hook", "secret");

    InvoiceWriter::upsert(&*ds, &fully_paid_invoice(&invoice_id, store_id))
        .await
        .unwrap();
    PaymentWriter::upsert(&*ds, &reorgable_payment(&invoice_id, "0xdoomed", 100))
        .await
        .unwrap();

    consumer
        .handle_reorg_detected(reorg_at(&invoice_id, 99))
        .await
        .unwrap();

    // Undo the marking to stand in for a crash between the write and the
    // enqueue, then let the handler run the same transition again.
    let mut replayed = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap()
        .remove(0);
    replayed.reorged = false;
    PaymentWriter::upsert(&*ds, &replayed).await.unwrap();
    InvoiceWriter::update_status(&*ds, &invoice_id, InvoiceStatus::Processing)
        .await
        .unwrap();

    consumer
        .handle_reorg_detected(reorg_at(&invoice_id, 99))
        .await
        .unwrap();

    let jobs = sink.jobs();
    assert_eq!(jobs.len(), 2);
    assert_eq!(
        jobs[0].payload.idempotency_key, jobs[1].payload.idempotency_key,
        "the same logical event must carry the same key"
    );
    assert_ne!(
        jobs[0].payload.event_id, jobs[1].payload.event_id,
        "event_id identifies the delivery, so it is not the thing to dedupe on"
    );
}

/// A reorg on this invoice at `fork_block`, on chain 1.
fn reorg_at(invoice_id: &InvoiceId, fork_block: u64) -> ReorgDetected {
    ReorgDetected {
        chain_id: 1,
        fork_block,
        old_hash: B256::ZERO,
        new_hash: B256::repeat_byte(0x01),
        depth: 2,
        affected_invoices: vec![Uuid::parse_str(invoice_id.as_str()).unwrap()],
        detected_at: Utc::now(),
    }
}

/// An unconfirmed native payment on chain 1 at `block`.
fn reorgable_payment(invoice_id: &InvoiceId, tx_hash: &str, block: u64) -> PaymentData {
    PaymentData {
        id: Uuid::new_v4(),
        invoice_id: invoice_id.clone(),
        payment_option_id: None,
        chain_id: ChainId::parse("eip155:1").unwrap(),
        asset_type: types::AssetType::Native,
        amount: "1000000000000000000".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        tx_hash: tx_hash.to_string(),
        block_number: Some(block),
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: Some("0xsender".to_string()),
        reorged: false,
        extra: None,
        credited_amount: Some("1.0".to_string()),
        rate_used: None,
        rate_applied_at: None,
    }
}
