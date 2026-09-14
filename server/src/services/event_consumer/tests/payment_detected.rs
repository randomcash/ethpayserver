#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::Utc;
use data_service::InMemoryDataService;
use evm::monitor::bridge::MemoryBridge;
use evm::monitor::events::PaymentDetected;
use evm::{Address, B256, U256};
use std::sync::Arc;
use types::ChainId;
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
    assert_eq!(payments[0].chain_id, ChainId::evm(99999));
    assert_eq!(payments[0].asset_symbol, "ETH"); // Fallback for unknown chains
}

/// RCS-282: a batching contract, a multicall, or an exchange sweep can settle
/// two transfers to two watched addresses in a single transaction. Both must
/// be recorded - before the fix, `payments` had no way to tell them apart
/// (`unique_payment_tx` was `(tx_hash, chain_id)` alone), so the second
/// `PaymentDetected` event's upsert silently overwrote the first instead of
/// inserting a second row.
#[tokio::test]
async fn test_handle_payment_detected_two_transfers_in_one_tx_both_survive() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds.clone(), bridge.clone());

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();
    create_test_invoice(&ds, &invoice_id, store_id).await;

    let shared_tx_hash = B256::repeat_byte(0xcd);

    let first_amount = U256::from(500000000000000000u64); // 0.5 ETH
    let second_amount = U256::from(300000000000000000u64); // 0.3 ETH

    let base_event = PaymentDetected {
        chain_id: 1,
        invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: Address::ZERO,
        amount: first_amount,
        tx_hash: shared_tx_hash,
        block_number: 12345678,
        block_hash: B256::ZERO,
        log_index: Some(0),
        is_native: true,
        token_address: None,
        from_address: Address::repeat_byte(0xab),
        confirmations: 1,
        required_confirmations: 12,
        detected_at: Utc::now(),
    };

    let first_event = base_event.clone();
    let second_event = PaymentDetected {
        payment_address: Address::repeat_byte(0x02),
        amount: second_amount,
        log_index: Some(1),
        from_address: Address::repeat_byte(0xef),
        ..base_event
    };

    consumer.handle_payment_detected(first_event).await.unwrap();
    consumer
        .handle_payment_detected(second_event)
        .await
        .unwrap();

    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert_eq!(
        payments.len(),
        2,
        "two transfers sharing a tx_hash must produce two payments, not one \
         overwritten by the other"
    );

    let amounts: std::collections::HashSet<_> = payments.iter().map(|p| p.amount.clone()).collect();
    assert!(amounts.contains(&first_amount.to_string()));
    assert!(amounts.contains(&second_amount.to_string()));
}

/// RCS-282 follow-up: a native transfer (`log_index: None`) and an ERC20
/// transfer whose *real* log index happens to be 0 can share a `tx_hash` -
/// e.g. a contract that receives ETH directly at the top level and, in the
/// same transaction, emits a Transfer log at index 0 to a different watched
/// address. If native transfers were keyed on `tx_index = 0`, this would
/// collide with exactly that ERC20 transfer and silently overwrite it. The
/// fix keys native transfers on a -1 sentinel instead, which no real log
/// index can ever produce.
#[tokio::test]
async fn test_handle_payment_detected_native_and_log_index_zero_both_survive() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds.clone(), bridge.clone());

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();
    create_test_invoice(&ds, &invoice_id, store_id).await;

    let shared_tx_hash = B256::repeat_byte(0xef);

    let native_amount = U256::from(500000000000000000u64); // 0.5 ETH
    let erc20_amount = U256::from(1_000_000u64);

    let native_event = PaymentDetected {
        chain_id: 1,
        invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: Address::repeat_byte(0x01),
        amount: native_amount,
        tx_hash: shared_tx_hash,
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

    let erc20_event = PaymentDetected {
        chain_id: 1,
        invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: Address::repeat_byte(0x02),
        amount: erc20_amount,
        tx_hash: shared_tx_hash,
        block_number: 12345678,
        block_hash: B256::ZERO,
        log_index: Some(0),
        is_native: false,
        token_address: Some(Address::repeat_byte(0x03)),
        from_address: Address::repeat_byte(0xef),
        confirmations: 1,
        required_confirmations: 12,
        detected_at: Utc::now(),
    };

    consumer
        .handle_payment_detected(native_event)
        .await
        .unwrap();
    consumer.handle_payment_detected(erc20_event).await.unwrap();

    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert_eq!(
        payments.len(),
        2,
        "a native transfer and an ERC20 transfer at log_index 0 sharing a \
         tx_hash must both survive, not collide on tx_index"
    );

    let amounts: std::collections::HashSet<_> = payments.iter().map(|p| p.amount.clone()).collect();
    assert!(amounts.contains(&native_amount.to_string()));
    assert!(amounts.contains(&erc20_amount.to_string()));
}
