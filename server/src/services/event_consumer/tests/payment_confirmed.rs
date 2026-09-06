#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::Utc;
use data_service::InMemoryDataService;
use evm::monitor::bridge::MemoryBridge;
use evm::monitor::events::PaymentConfirmed;
use evm::{Address, B256, U256};
use std::sync::Arc;
use types::{
    InvoiceData, InvoiceId, InvoiceReader, InvoiceStatus, InvoiceWriter, PaymentData,
    PaymentReader, PaymentWriter, StoreId,
};
use uuid::Uuid;

use super::helpers::{MockEVMMonitor, MockEmailSender, create_test_consumer};
use crate::services::event_consumer::EventConsumer;

#[tokio::test]
async fn test_handle_payment_confirmed_transitions_to_paid() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds.clone(), bridge.clone());

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();

    // Create invoice in processing state with full amount received
    let invoice = InvoiceData {
        id: invoice_id.clone(),
        store_id,
        currency: "ETH".to_string(),
        status: InvoiceStatus::Processing,
        amount: "1000000000000000000".to_string(), // 1 ETH
        amount_received: "1000000000000000000".to_string(), // 1 ETH received
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        customer_email: None,
        extra: None,
    };
    InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

    // Create a payment record
    let tx_hash = B256::repeat_byte(0xab);
    let payment = PaymentData {
        id: Uuid::new_v4(),
        invoice_id: invoice_id.clone(),
        payment_option_id: None,
        chain_id: 1,
        asset_type: types::AssetType::Native,
        amount: "1000000000000000000".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        tx_hash: format!("{:#x}", tx_hash),
        block_number: Some(12345678),
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: Some("0xabababababababababababababababababababab".to_string()),
        reorged: false,
        extra: None,
        credited_amount: Some("1".to_string()), // 1 ETH
        rate_used: None,
        rate_applied_at: None,
    };
    PaymentWriter::upsert(&*ds, &payment).await.unwrap();

    // Create PaymentConfirmed event
    let event = PaymentConfirmed {
        chain_id: 1,
        invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: Address::ZERO,
        amount: U256::from(1000000000000000000u64),
        tx_hash,
        block_number: 12345678,
        confirmations: 12,
        confirmed_at: Utc::now(),
    };

    // Handle the event
    consumer.handle_payment_confirmed(event).await.unwrap();

    // Verify invoice was marked as paid
    let invoice = InvoiceReader::get(&*ds, &invoice_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(invoice.status, InvoiceStatus::Paid);

    // Verify payment was marked as confirmed
    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert!(payments[0].confirmed_at.is_some());
}

#[tokio::test]
async fn test_handle_payment_confirmed_skips_cancelled_invoice() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds.clone(), bridge.clone());

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();

    // Create cancelled invoice
    let invoice = InvoiceData {
        id: invoice_id.clone(),
        store_id,
        currency: "ETH".to_string(),
        status: InvoiceStatus::Cancelled,
        amount: "1000000000000000000".to_string(),
        amount_received: "1000000000000000000".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        customer_email: None,
        extra: None,
    };
    InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

    // Create a payment record
    let tx_hash = B256::repeat_byte(0xab);
    let payment = PaymentData {
        id: Uuid::new_v4(),
        invoice_id: invoice_id.clone(),
        payment_option_id: None,
        chain_id: 1,
        asset_type: types::AssetType::Native,
        amount: "1000000000000000000".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        tx_hash: format!("{:#x}", tx_hash),
        block_number: Some(12345678),
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: None,
        reorged: false,
        extra: None,
        credited_amount: Some("1".to_string()),
        rate_used: None,
        rate_applied_at: None,
    };
    PaymentWriter::upsert(&*ds, &payment).await.unwrap();

    // Create PaymentConfirmed event
    let event = PaymentConfirmed {
        chain_id: 1,
        invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: Address::ZERO,
        amount: U256::from(1000000000000000000u64),
        tx_hash,
        block_number: 12345678,
        confirmations: 12,
        confirmed_at: Utc::now(),
    };

    // Handle the event
    consumer.handle_payment_confirmed(event).await.unwrap();

    // Invoice should still be cancelled
    let invoice = InvoiceReader::get(&*ds, &invoice_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(invoice.status, InvoiceStatus::Cancelled);
}

#[tokio::test]
async fn test_handle_payment_confirmed_late_payment_on_expired_invoice() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds.clone(), bridge.clone());

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();

    // Create an expired invoice (with full amount received - late payment scenario)
    let invoice = InvoiceData {
        id: invoice_id.clone(),
        store_id,
        currency: "ETH".to_string(),
        status: InvoiceStatus::Expired,
        amount: "1000000000000000000".to_string(), // 1 ETH
        amount_received: "1000000000000000000".to_string(), // Full amount received after expiry
        created_at: Utc::now() - chrono::Duration::hours(2),
        expires_at: Utc::now() - chrono::Duration::hours(1), // Expired an hour ago
        metadata: None,
        customer_email: None,
        extra: None,
    };
    InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

    // Create a payment record (detected after expiry)
    let tx_hash = B256::repeat_byte(0xcc);
    let payment = PaymentData {
        id: Uuid::new_v4(),
        invoice_id: invoice_id.clone(),
        payment_option_id: None,
        chain_id: 1,
        asset_type: types::AssetType::Native,
        amount: "1000000000000000000".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        tx_hash: format!("{:#x}", tx_hash),
        block_number: Some(12345700),
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: Some("0xcccccccccccccccccccccccccccccccccccccccc".to_string()),
        reorged: false,
        extra: None,
        credited_amount: Some("1".to_string()),
        rate_used: None,
        rate_applied_at: None,
    };
    PaymentWriter::upsert(&*ds, &payment).await.unwrap();

    // Create PaymentConfirmed event for the late payment
    let event = PaymentConfirmed {
        chain_id: 1,
        invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: Address::ZERO,
        amount: U256::from(1000000000000000000u64),
        tx_hash,
        block_number: 12345700,
        confirmations: 12,
        confirmed_at: Utc::now(),
    };

    // Handle the event
    consumer.handle_payment_confirmed(event).await.unwrap();

    // Verify invoice was marked as LatePaid (not Paid)
    let invoice = InvoiceReader::get(&*ds, &invoice_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(invoice.status, InvoiceStatus::LatePaid);

    // Verify payment was marked as confirmed
    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert!(payments[0].confirmed_at.is_some());
}

#[tokio::test]
async fn test_receipt_sent_on_paid_with_email() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let mock_email = Arc::new(MockEmailSender::new());
    let consumer: EventConsumer<_, MockEVMMonitor> = EventConsumer::new(
        bridge.clone(),
        ds.clone(),
        None,
        None,
        None,
        mock_email.clone(),
    );

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();

    // Legacy path: address inside metadata, no column. Covers invoices created
    // before RCS-215; new writes populate the column instead.
    let invoice = InvoiceData {
        id: invoice_id.clone(),
        store_id,
        currency: "USD".to_string(),
        status: InvoiceStatus::Processing,
        amount: "100.00".to_string(),
        amount_received: "100.00".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: Some(serde_json::json!({"customer_email": "buyer@example.com"})),
        customer_email: None,
        extra: None,
    };
    InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

    // Create payment
    let tx_hash = B256::repeat_byte(0xee);
    let payment = PaymentData {
        id: Uuid::new_v4(),
        invoice_id: invoice_id.clone(),
        payment_option_id: None,
        chain_id: 1,
        asset_type: types::AssetType::Native,
        amount: "50000000000000000".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        tx_hash: format!("{:#x}", tx_hash),
        block_number: Some(12346000),
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: Some("0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_string()),
        reorged: false,
        extra: None,
        credited_amount: Some("100.00".to_string()),
        rate_used: Some("2000.00".to_string()),
        rate_applied_at: Some(Utc::now()),
    };
    PaymentWriter::upsert(&*ds, &payment).await.unwrap();

    // Handle payment confirmed event
    let event = PaymentConfirmed {
        chain_id: 1,
        invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: Address::ZERO,
        amount: U256::from(50000000000000000u64),
        tx_hash,
        block_number: 12346000,
        confirmations: 12,
        confirmed_at: Utc::now(),
    };

    consumer.handle_payment_confirmed(event).await.unwrap();

    // Verify receipt email was sent
    assert_eq!(mock_email.call_count(), 1);
    let calls = mock_email.calls();
    assert_eq!(calls[0].0, "buyer@example.com");
    assert_eq!(calls[0].1, invoice_id.as_str());
}

#[tokio::test]
async fn test_no_receipt_when_email_absent() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let mock_email = Arc::new(MockEmailSender::new());
    let consumer: EventConsumer<_, MockEVMMonitor> = EventConsumer::new(
        bridge.clone(),
        ds.clone(),
        None,
        None,
        None,
        mock_email.clone(),
    );

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();

    // Create invoice WITHOUT customer_email
    let invoice = InvoiceData {
        id: invoice_id.clone(),
        store_id,
        currency: "USD".to_string(),
        status: InvoiceStatus::Processing,
        amount: "100.00".to_string(),
        amount_received: "100.00".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        customer_email: None,
        extra: None,
    };
    InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

    // Create payment
    let tx_hash = B256::repeat_byte(0xff);
    let payment = PaymentData {
        id: Uuid::new_v4(),
        invoice_id: invoice_id.clone(),
        payment_option_id: None,
        chain_id: 1,
        asset_type: types::AssetType::Native,
        amount: "50000000000000000".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        tx_hash: format!("{:#x}", tx_hash),
        block_number: Some(12347000),
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: Some("0xffffffffffffffffffffffffffffffffffffffff".to_string()),
        reorged: false,
        extra: None,
        credited_amount: Some("100.00".to_string()),
        rate_used: Some("2000.00".to_string()),
        rate_applied_at: Some(Utc::now()),
    };
    PaymentWriter::upsert(&*ds, &payment).await.unwrap();

    // Handle payment confirmed event
    let event = PaymentConfirmed {
        chain_id: 1,
        invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: Address::ZERO,
        amount: U256::from(50000000000000000u64),
        tx_hash,
        block_number: 12347000,
        confirmations: 12,
        confirmed_at: Utc::now(),
    };

    consumer.handle_payment_confirmed(event).await.unwrap();

    // Verify NO receipt email was sent
    assert_eq!(mock_email.call_count(), 0);
}

/// RCS-215: receipts must read the `customer_email` column, not just metadata.
///
/// This is the path every invoice created after RCS-215 takes - the address is
/// written to its own column and deliberately kept out of `metadata`, which is
/// slated to become ciphertext (RCS-216). Before the column was threaded
/// through, `extract_customer_email` looked only at metadata, so this case
/// returned None and the receipt was silently skipped: a missing address is a
/// normal, unlogged outcome there, so nothing would have reported the breakage.
#[tokio::test]
async fn test_receipt_sent_from_customer_email_column() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let mock_email = Arc::new(MockEmailSender::new());
    let consumer: EventConsumer<_, MockEVMMonitor> = EventConsumer::new(
        bridge.clone(),
        ds.clone(),
        None,
        None,
        None,
        mock_email.clone(),
    );

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();

    // Column set, metadata empty - the inverse of the legacy test above.
    let invoice = InvoiceData {
        id: invoice_id.clone(),
        store_id,
        currency: "USD".to_string(),
        status: InvoiceStatus::Processing,
        amount: "100.00".to_string(),
        amount_received: "100.00".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        customer_email: Some("column@example.com".to_string()),
        extra: None,
    };
    InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

    let tx_hash = B256::repeat_byte(0xcc);
    let payment = PaymentData {
        id: Uuid::new_v4(),
        invoice_id: invoice_id.clone(),
        payment_option_id: None,
        chain_id: 1,
        asset_type: types::AssetType::Native,
        amount: "50000000000000000".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        tx_hash: format!("{:#x}", tx_hash),
        block_number: Some(12347000),
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: Some("0xcccccccccccccccccccccccccccccccccccccccc".to_string()),
        reorged: false,
        extra: None,
        credited_amount: Some("100.00".to_string()),
        rate_used: Some("2000.00".to_string()),
        rate_applied_at: Some(Utc::now()),
    };
    PaymentWriter::upsert(&*ds, &payment).await.unwrap();

    consumer
        .handle_payment_confirmed(PaymentConfirmed {
            chain_id: 1,
            invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
            payment_address: Address::ZERO,
            amount: U256::from(50000000000000000u64),
            tx_hash,
            block_number: 12347000,
            confirmations: 12,
            confirmed_at: Utc::now(),
        })
        .await
        .unwrap();

    assert_eq!(
        mock_email.call_count(),
        1,
        "receipt must be sent from the column"
    );
    assert_eq!(mock_email.calls()[0].0, "column@example.com");
}
