#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::services::email;
use async_trait::async_trait;
use chrono::Utc;
use data_service::InMemoryDataService;
use evm::monitor::bridge::MemoryBridge;
use evm::monitor::events::{PaymentConfirmed, PaymentDetected, ReorgDetected};
use evm::{Address, B256, U256};
use std::sync::Arc;
use types::{
    InvoiceData, InvoiceId, InvoiceStatus, InvoiceWriter, Network, PaymentData, PaymentReader,
    PaymentWriter, StoreId,
};
use uuid::Uuid;

use super::super::evm_monitor::{EVMMonitor, EVMMonitorError};

/// Get the native asset symbol for a network (test helper).
fn network_native_symbol(network: types::Network) -> String {
    use types::Network::*;
    match network {
        Ethereum | Arbitrum | Optimism | Base | ZkSync | Linea | Scroll => "ETH",
        Polygon => "POL",
        Avalanche => "AVAX",
        BinanceSmartChain => "BNB",
        Fantom => "FTM",
        Gnosis => "xDAI",
        // Non-EVM networks - shouldn't reach here
        _ => "UNKNOWN",
    }
    .to_string()
}

/// Mock EVMMonitor for testing.
struct MockEVMMonitor;

#[async_trait]
impl EVMMonitor for MockEVMMonitor {
    async fn watch_address(
        &self,
        _network: Network,
        _address: Address,
        _invoice_id: Uuid,
        _expected_amount: Option<U256>,
        _token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        Ok(())
    }

    async fn watch_address_by_chain_id(
        &self,
        _chain_id: u64,
        _address: Address,
        _invoice_id: Uuid,
        _expected_amount: Option<U256>,
        _token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        Ok(())
    }

    async fn unwatch_address(
        &self,
        _network: Network,
        _address: Address,
        _token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        Ok(())
    }

    async fn unwatch_address_by_chain_id(
        &self,
        _chain_id: u64,
        _address: Address,
        _token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        Ok(())
    }

    async fn health_check(&self) -> Result<(), EVMMonitorError> {
        Ok(())
    }

    async fn get_chain_health(&self) -> Result<Vec<evm::monitor::ChainHealth>, EVMMonitorError> {
        Ok(vec![])
    }
}

/// Create a test invoice in the data service.
async fn create_test_invoice(ds: &InMemoryDataService, invoice_id: &InvoiceId, store_id: StoreId) {
    let invoice = InvoiceData {
        id: invoice_id.clone(),
        store_id,
        currency: "ETH".to_string(),
        status: InvoiceStatus::Pending,
        amount: "1000000000000000000".to_string(), // 1 ETH
        amount_received: "0".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        extra: None,
    };
    InvoiceWriter::upsert(ds, &invoice).await.unwrap();
}

#[test]
fn test_network_native_symbol() {
    assert_eq!(network_native_symbol(Network::Ethereum), "ETH");
    assert_eq!(network_native_symbol(Network::Polygon), "POL");
    assert_eq!(network_native_symbol(Network::Avalanche), "AVAX");
    assert_eq!(network_native_symbol(Network::BinanceSmartChain), "BNB");
    assert_eq!(network_native_symbol(Network::Arbitrum), "ETH");
    assert_eq!(network_native_symbol(Network::Optimism), "ETH");
    assert_eq!(network_native_symbol(Network::Base), "ETH");
    assert_eq!(network_native_symbol(Network::Fantom), "FTM");
    assert_eq!(network_native_symbol(Network::Gnosis), "xDAI");
    // Non-EVM networks
    assert_eq!(network_native_symbol(Network::BitcoinMainnet), "UNKNOWN");
}

#[tokio::test]
async fn test_handle_payment_detected_native() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());

    let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> = EventConsumer::new(
        bridge.clone(),
        ds.clone(),
        None,
        None,
        None,
        Arc::new(email::NoopEmailSender),
    );

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
        Arc::new(email::NoopEmailSender),
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

#[tokio::test]
async fn test_handle_payment_confirmed_transitions_to_paid() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());

    let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> = EventConsumer::new(
        bridge.clone(),
        ds.clone(),
        None,
        None,
        None,
        Arc::new(email::NoopEmailSender),
    );

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

    let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> = EventConsumer::new(
        bridge.clone(),
        ds.clone(),
        None,
        None,
        None,
        Arc::new(email::NoopEmailSender),
    );

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
async fn test_handle_reorg_detected() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());

    let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> = EventConsumer::new(
        bridge.clone(),
        ds.clone(),
        None,
        None,
        None,
        Arc::new(email::NoopEmailSender),
    );

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
#[allow(clippy::too_many_lines)] // multi-step reorg test with setup + multiple assertions
async fn test_handle_reorg_with_remaining_valid_payments() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());

    let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> = EventConsumer::new(
        bridge.clone(),
        ds.clone(),
        None,
        None,
        None,
        Arc::new(email::NoopEmailSender),
    );

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();

    // Create invoice in processing state
    let invoice = InvoiceData {
        id: invoice_id.clone(),
        store_id,
        currency: "ETH".to_string(),
        status: InvoiceStatus::Processing,
        amount: "1000000000000000000".to_string(),
        amount_received: "1000000000000000000".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        extra: None,
    };
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

#[tokio::test]
async fn test_handle_payment_confirmed_late_payment_on_expired_invoice() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());

    let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> = EventConsumer::new(
        bridge.clone(),
        ds.clone(),
        None,
        None,
        None,
        Arc::new(email::NoopEmailSender),
    );

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

#[test]
fn test_event_consumer_error_display() {
    let db_err = EventConsumerError::Database(types::RepositoryError::NotFound("test".into()));
    assert!(db_err.to_string().contains("database error"));

    let data_err = EventConsumerError::InvalidData("bad data".into());
    assert!(data_err.to_string().contains("invalid data"));
}

/// Mock email sender that records calls for test assertions.
struct MockEmailSender {
    calls: std::sync::Mutex<Vec<(String, String)>>, // (to, invoice_id)
}

impl MockEmailSender {
    fn new() -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

#[async_trait]
impl email::EmailSender for MockEmailSender {
    async fn send_receipt(
        &self,
        to: &str,
        data: &email::ReceiptData,
    ) -> Result<(), email::EmailError> {
        self.calls
            .lock()
            .unwrap()
            .push((to.to_string(), data.invoice_id.clone()));
        Ok(())
    }
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

    // Create invoice with customer_email in metadata
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
    let calls = mock_email.calls.lock().unwrap();
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
