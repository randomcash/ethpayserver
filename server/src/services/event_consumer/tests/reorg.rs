#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::Utc;
use data_service::InMemoryDataService;
use evm::B256;
use evm::monitor::bridge::MemoryBridge;
use evm::monitor::events::ReorgDetected;
use std::sync::Arc;
use types::{
    InvoiceData, InvoiceId, InvoiceReader, InvoiceStatus, InvoiceWriter, PaymentData,
    PaymentReader, PaymentWriter, StoreId,
};
use uuid::Uuid;

use super::helpers::create_test_consumer;

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
        chain_id: 1,
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
        chain_id: 1,
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
        chain_id: 1,
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
