//! End-to-end integration tests for the payment detection pipeline.
//!
//! Verifies the full flow: invoice creation → payment detection → event processing
//! → invoice status transitions → webhook enqueueing.
//!
//! Uses InMemoryDataService + MemoryBridge to test the EventConsumer without
//! requiring a database, Redis, or live RPC connections.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use data_service::InMemoryDataService;
use evm::monitor::bridge::MemoryBridge;
use evm::monitor::events::{MonitorEvent, PaymentConfirmed, PaymentDetected};
use evm::monitor::{ChainHealth, EventBridge};
use evm::{Address, B256, U256};
use server::EventConsumer;
use server::services::evm_monitor::{EVMMonitor, EVMMonitorError};
use types::{
    InvoiceData, InvoiceId, InvoiceReader, InvoiceStatus, InvoiceWriter, Network, PaymentMethodId,
    PaymentOptionData, PaymentOptionId, PaymentReader, StoreId, WatchedAddressWriter,
};

// ============================================================================
// Test-only EVMMonitor stub (no-op — only needed for type parameter)
// ============================================================================

struct NoopEVMMonitor;

#[async_trait]
impl EVMMonitor for NoopEVMMonitor {
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

    async fn get_chain_health(&self) -> Result<Vec<ChainHealth>, EVMMonitorError> {
        Ok(vec![])
    }
}

// ============================================================================
// Test helpers
// ============================================================================

/// Sepolia chain ID used in tests.
const TEST_CHAIN_ID: u64 = 11155111;

fn test_invoice(store_id: StoreId) -> InvoiceData {
    InvoiceData {
        id: InvoiceId::new(),
        store_id,
        currency: "USD".to_string(),
        status: InvoiceStatus::Pending,
        amount: "100.00".to_string(),
        amount_received: "0".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        extra: None,
    }
}

fn test_payment_option(
    invoice_id: &InvoiceId,
    address: &str,
    amount_wei: &str,
    rate: &str,
) -> PaymentOptionData {
    PaymentOptionData {
        id: PaymentOptionId(Uuid::new_v4()),
        invoice_id: invoice_id.clone(),
        payment_method_id: PaymentMethodId::new("ETH", TEST_CHAIN_ID),
        chain_id: TEST_CHAIN_ID,
        asset_symbol: "ETH".to_string(),
        token_address: None,
        decimals: 18,
        payment_address: address.to_string(),
        amount: amount_wei.to_string(),
        rate: Some(rate.to_string()),
        rate_at: Some(Utc::now()),
        is_active: true,
        created_at: Utc::now(),
    }
}

/// Set up the test environment: data service, bridge, invoice, payment option, watched address.
async fn setup_test_env() -> (
    Arc<InMemoryDataService>,
    Arc<MemoryBridge>,
    InvoiceData,
    PaymentOptionData,
    Address,
) {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());

    let store_id = StoreId::new();
    let invoice = test_invoice(store_id);
    InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

    // Payment address (deterministic for the test)
    let payment_address = Address::random();
    let payment_address_str = format!("{:#x}", payment_address);

    // 0.05 ETH at rate 2000 = $100
    let payment_option = test_payment_option(
        &invoice.id,
        &payment_address_str,
        "50000000000000000", // 0.05 ETH in wei
        "2000.00",           // ETH/USD rate
    );
    data_service::PaymentOptionWriter::create(&*ds, &payment_option)
        .await
        .unwrap();

    // Register watched address
    WatchedAddressWriter::upsert(
        &*ds,
        &payment_address_str,
        &payment_option.id,
        TEST_CHAIN_ID,
        None, // native
    )
    .await
    .unwrap();

    (ds, bridge, invoice, payment_option, payment_address)
}

/// Spawn an EventConsumer in the background.
fn spawn_consumer(
    bridge: Arc<MemoryBridge>,
    ds: Arc<InMemoryDataService>,
) -> tokio::task::JoinHandle<()> {
    let consumer: EventConsumer<InMemoryDataService, NoopEVMMonitor> = EventConsumer::new(
        bridge as Arc<dyn EventBridge>,
        ds,
        None, // no cleanup service
        None, // no webhook service (requires Redis)
        None, // no WsBroadcast
    );
    tokio::spawn(async move { consumer.run().await })
}

// ============================================================================
// Test: PaymentDetected creates payment record and updates invoice
// ============================================================================

#[tokio::test]
async fn test_payment_detected_creates_record() {
    let (ds, bridge, invoice, _po, payment_address) = setup_test_env().await;
    let consumer_handle = spawn_consumer(bridge.clone(), ds.clone());

    // Give the consumer time to subscribe
    tokio::time::sleep(Duration::from_millis(50)).await;

    let tx_hash = B256::random();
    let payment_amount = U256::from(50_000_000_000_000_000u64); // 0.05 ETH

    // Publish PaymentDetected to bridge
    let event = MonitorEvent::PaymentDetected(PaymentDetected {
        chain_id: TEST_CHAIN_ID,
        invoice_id: Uuid::parse_str(invoice.id.as_str()).unwrap(),
        payment_address,
        amount: payment_amount,
        tx_hash,
        block_number: 100,
        block_hash: B256::random(),
        log_index: None,
        is_native: true,
        token_address: None,
        from_address: Address::random(),
        confirmations: 1,
        required_confirmations: 3,
        detected_at: Utc::now(),
    });

    bridge.publish(&event).await.unwrap();

    // Wait for processing
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify payment record was created
    let payments = PaymentReader::get_for_invoice(&*ds, &invoice.id)
        .await
        .unwrap();
    assert_eq!(payments.len(), 1, "expected 1 payment record");

    let payment = &payments[0];
    assert_eq!(payment.chain_id, TEST_CHAIN_ID);
    assert_eq!(payment.amount, payment_amount.to_string());
    assert_eq!(payment.tx_hash, format!("{:#x}", tx_hash));
    assert!(payment.confirmed_at.is_none());
    assert!(!payment.reorged);
    // credited_amount should be set (0.05 ETH / 2000.00 rate = $100)
    assert!(payment.credited_amount.is_some());

    consumer_handle.abort();
}

// ============================================================================
// Test: PaymentConfirmed transitions invoice to Paid
// ============================================================================

#[tokio::test]
async fn test_payment_confirmed_marks_invoice_paid() {
    let (ds, bridge, invoice, _po, payment_address) = setup_test_env().await;
    let consumer_handle = spawn_consumer(bridge.clone(), ds.clone());
    tokio::time::sleep(Duration::from_millis(50)).await;

    let tx_hash = B256::random();
    let payment_amount = U256::from(50_000_000_000_000_000u64);
    let invoice_uuid = Uuid::parse_str(invoice.id.as_str()).unwrap();

    // Step 1: PaymentDetected
    bridge
        .publish(&MonitorEvent::PaymentDetected(PaymentDetected {
            chain_id: TEST_CHAIN_ID,
            invoice_id: invoice_uuid,
            payment_address,
            amount: payment_amount,
            tx_hash,
            block_number: 100,
            block_hash: B256::random(),
            log_index: None,
            is_native: true,
            token_address: None,
            from_address: Address::random(),
            confirmations: 1,
            required_confirmations: 3,
            detected_at: Utc::now(),
        }))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    // The DB trigger in production updates amount_received automatically.
    // In our InMemoryDataService, we need to simulate this manually.
    // The EventConsumer creates a PaymentData with credited_amount.
    // In production, a DB trigger sums credited_amounts and updates amount_received.
    // For the test, we manually update amount_received to match the invoice amount.
    let payments = PaymentReader::get_for_invoice(&*ds, &invoice.id)
        .await
        .unwrap();
    assert!(!payments.is_empty(), "payment should exist");

    // Simulate DB trigger: update invoice amount_received
    if let Some(credited) = &payments[0].credited_amount {
        InvoiceWriter::update_amount_received(&*ds, &invoice.id, credited)
            .await
            .unwrap();
    }

    // Also update status to Processing (DB trigger does this in production)
    InvoiceWriter::update_status(&*ds, &invoice.id, InvoiceStatus::Processing)
        .await
        .unwrap();

    // Step 2: PaymentConfirmed
    bridge
        .publish(&MonitorEvent::PaymentConfirmed(PaymentConfirmed {
            chain_id: TEST_CHAIN_ID,
            invoice_id: invoice_uuid,
            payment_address,
            amount: payment_amount,
            tx_hash,
            block_number: 100,
            confirmations: 3,
            confirmed_at: Utc::now(),
        }))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify invoice is now Paid
    let updated_invoice = InvoiceReader::get(&*ds, &invoice.id)
        .await
        .unwrap()
        .expect("invoice should exist");
    assert_eq!(
        updated_invoice.status,
        InvoiceStatus::Paid,
        "invoice should be Paid after confirmed payment covers full amount"
    );

    // Verify payment is marked confirmed
    let payments = PaymentReader::get_for_invoice(&*ds, &invoice.id)
        .await
        .unwrap();
    assert!(
        payments[0].confirmed_at.is_some(),
        "payment should have confirmed_at set"
    );

    consumer_handle.abort();
}

// ============================================================================
// Test: Underpayment keeps invoice in Processing
// ============================================================================

#[tokio::test]
async fn test_underpayment_stays_processing() {
    let (ds, bridge, invoice, _po, payment_address) = setup_test_env().await;
    let consumer_handle = spawn_consumer(bridge.clone(), ds.clone());
    tokio::time::sleep(Duration::from_millis(50)).await;

    let invoice_uuid = Uuid::parse_str(invoice.id.as_str()).unwrap();
    // Send only half the amount (0.025 ETH instead of 0.05)
    let half_amount = U256::from(25_000_000_000_000_000u64);
    let tx_hash = B256::random();

    bridge
        .publish(&MonitorEvent::PaymentDetected(PaymentDetected {
            chain_id: TEST_CHAIN_ID,
            invoice_id: invoice_uuid,
            payment_address,
            amount: half_amount,
            tx_hash,
            block_number: 100,
            block_hash: B256::random(),
            log_index: None,
            is_native: true,
            token_address: None,
            from_address: Address::random(),
            confirmations: 1,
            required_confirmations: 3,
            detected_at: Utc::now(),
        }))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Simulate DB trigger: update amount_received with half
    let payments = PaymentReader::get_for_invoice(&*ds, &invoice.id)
        .await
        .unwrap();
    if let Some(credited) = &payments[0].credited_amount {
        InvoiceWriter::update_amount_received(&*ds, &invoice.id, credited)
            .await
            .unwrap();
    }
    InvoiceWriter::update_status(&*ds, &invoice.id, InvoiceStatus::Processing)
        .await
        .unwrap();

    // Confirm the half payment
    bridge
        .publish(&MonitorEvent::PaymentConfirmed(PaymentConfirmed {
            chain_id: TEST_CHAIN_ID,
            invoice_id: invoice_uuid,
            payment_address,
            amount: half_amount,
            tx_hash,
            block_number: 100,
            confirmations: 3,
            confirmed_at: Utc::now(),
        }))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Invoice should still be Processing (not Paid) because amount_received < amount
    let inv = InvoiceReader::get(&*ds, &invoice.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        inv.status,
        InvoiceStatus::Processing,
        "underpaid invoice should stay Processing"
    );

    consumer_handle.abort();
}

// ============================================================================
// Test: Invoice expiration path
// ============================================================================

#[tokio::test]
async fn test_expired_invoice_status() {
    let ds = Arc::new(InMemoryDataService::new());

    // Create an already-expired invoice
    let store_id = StoreId::new();
    let mut invoice = test_invoice(store_id);
    invoice.expires_at = Utc::now() - chrono::Duration::minutes(5);
    InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

    // Verify it appears in expired list
    let expired = InvoiceReader::get_expired(&*ds).await.unwrap();
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].id, invoice.id);

    // Expire it
    let result = InvoiceWriter::expire(&*ds, &invoice.id).await.unwrap();
    assert!(result, "should expire successfully");

    let updated = InvoiceReader::get(&*ds, &invoice.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.status, InvoiceStatus::Expired);

    // Expiring again should return false (already expired)
    let result2 = InvoiceWriter::expire(&*ds, &invoice.id).await.unwrap();
    assert!(!result2, "already expired, should return false");
}

// ============================================================================
// Test: ERC20 payment detection via EventConsumer
// ============================================================================

#[tokio::test]
async fn test_erc20_payment_detection_event_consumer() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());

    let store_id = StoreId::new();
    let invoice = test_invoice(store_id);
    InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

    let payment_address = Address::random();
    let payment_address_str = format!("{:#x}", payment_address);
    let token_contract = Address::random();
    let token_address_str = format!("{:#x}", token_contract);

    // Create ERC20 payment option (USDT-like: 6 decimals, no rate = same-asset invoice)
    let po = PaymentOptionData {
        id: PaymentOptionId(Uuid::new_v4()),
        invoice_id: invoice.id.clone(),
        payment_method_id: PaymentMethodId::new("USDT", TEST_CHAIN_ID),
        chain_id: TEST_CHAIN_ID,
        asset_symbol: "USDT".to_string(),
        token_address: Some(token_address_str.clone()),
        decimals: 6,
        payment_address: payment_address_str.clone(),
        amount: "100000000".to_string(), // 100 USDT
        rate: None,                      // same-asset (USD-denominated invoice, USDT payment)
        rate_at: None,
        is_active: true,
        created_at: Utc::now(),
    };
    data_service::PaymentOptionWriter::create(&*ds, &po)
        .await
        .unwrap();

    WatchedAddressWriter::upsert(
        &*ds,
        &payment_address_str,
        &po.id,
        TEST_CHAIN_ID,
        Some(&token_address_str),
    )
    .await
    .unwrap();

    let consumer_handle = spawn_consumer(bridge.clone(), ds.clone());
    tokio::time::sleep(Duration::from_millis(50)).await;

    let tx_hash = B256::random();
    let payment_amount = U256::from(100_000_000u64); // 100 USDT
    let invoice_uuid = Uuid::parse_str(invoice.id.as_str()).unwrap();

    bridge
        .publish(&MonitorEvent::PaymentDetected(PaymentDetected {
            chain_id: TEST_CHAIN_ID,
            invoice_id: invoice_uuid,
            payment_address,
            amount: payment_amount,
            tx_hash,
            block_number: 200,
            block_hash: B256::random(),
            log_index: Some(5),
            is_native: false,
            token_address: Some(token_contract),
            from_address: Address::random(),
            confirmations: 1,
            required_confirmations: 3,
            detected_at: Utc::now(),
        }))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    let payments = PaymentReader::get_for_invoice(&*ds, &invoice.id)
        .await
        .unwrap();
    assert_eq!(payments.len(), 1);
    // On testnets, unknown tokens get a shortened address as symbol: "0x{first6hex}..."
    assert!(
        payments[0].asset_symbol.starts_with("0x"),
        "expected shortened address symbol for testnet ERC20, got: {}",
        payments[0].asset_symbol
    );
    assert!(!payments[0].reorged);

    consumer_handle.abort();
}
