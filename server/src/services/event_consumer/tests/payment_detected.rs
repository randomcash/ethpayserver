#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::Utc;
use data_service::InMemoryDataService;
use evm::monitor::bridge::MemoryBridge;
use evm::monitor::events::PaymentDetected;
use evm::{Address, B256, U256};
use std::sync::Arc;
use types::{InvoiceData, InvoiceId, InvoiceStatus, InvoiceWriter, PaymentReader, StoreId};

use super::helpers::{MockEVMMonitor, create_test_consumer, create_test_invoice};
use crate::services::event_consumer::EventConsumer;

#[tokio::test]
async fn test_handle_payment_detected_native() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds.clone(), bridge.clone());

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();
    create_test_invoice(&ds, &invoice_id, store_id).await;

    // Create PaymentDetected event
    let event = PaymentDetected {
        chain_id: 1, // Ethereum mainnet
        invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: Address::ZERO,
        amount: U256::from(500000000000000000u64), // 0.5 ETH
        tx_hash: B256::ZERO,
        block_number: 12345678,
        block_hash: B256::ZERO,
        log_index: None,
        is_native: true,
        token_address: None,
        from_address: Address::repeat_byte(0xab),
        confirmations: 1,
        required_confirmations: 12,
        detected_at: Utc::now(),
    };

    // Handle the event
    consumer.handle_payment_detected(event).await.unwrap();

    // Verify payment was created
    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert_eq!(payments.len(), 1);
    assert_eq!(payments[0].asset_symbol, "ETH");
    assert_eq!(payments[0].amount, "500000000000000000");
    assert!(!payments[0].reorged);
}

#[tokio::test]
async fn test_handle_payment_detected_unknown_chain() {
    // With network-agnostic approach, payments from unknown chains are accepted
    // chain_id is stored on the payment, not the invoice
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());

    let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> = EventConsumer::new(
        bridge.clone(),
        ds.clone(),
        None,
        None,
        None,
        Arc::new(crate::services::email::NoopEmailSender),
    );

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();

    // Create network-agnostic invoice
    let invoice = InvoiceData {
        id: invoice_id.clone(),
        store_id,
        currency: "ETH".to_string(),
        status: InvoiceStatus::Pending,
        amount: "1000000000000000000".to_string(),
        amount_received: "0".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        customer_email: None,
        extra: None,
    };
    InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

    // Create PaymentDetected event with unknown chain
    let event = PaymentDetected {
        chain_id: 99999, // Unknown chain
        invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: Address::ZERO,
        amount: U256::from(1000000u64),
        tx_hash: B256::ZERO,
        block_number: 12345678,
        block_hash: B256::ZERO,
        log_index: None,
        is_native: true,
        token_address: None,
        from_address: Address::ZERO,
        confirmations: 1,
        required_confirmations: 12,
        detected_at: Utc::now(),
    };

    // Should succeed - unknown chains are now accepted
    consumer.handle_payment_detected(event).await.unwrap();

    // Verify payment was created with chain_id
    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert_eq!(payments.len(), 1);
    assert_eq!(payments[0].chain_id, 99999);
    assert_eq!(payments[0].asset_symbol, "ETH"); // Fallback for unknown chains
}
