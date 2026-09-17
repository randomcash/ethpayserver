//! Capability 4 reached through the handler that actually dispatches it.
//!
//! `plugins::payment_observer`'s own tests call `notify_own_store_payment`
//! directly, which proves the dispatcher behaves — and proves nothing about
//! whether anything calls it. Three separate pieces of this server have
//! shipped fully tested and wired to nothing; these tests exist so this is not
//! the fourth. Delete the dispatch from `confirmation_handler` and every test
//! here fails while every test in `payment_observer` still passes.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::Utc;
use data_service::InMemoryDataService;
use evm::monitor::bridge::MemoryBridge;
use evm::monitor::events::PaymentConfirmed;
use evm::{Address, B256, U256};
use std::sync::Arc;
use types::{
    ChainId, InvoiceData, InvoiceId, InvoiceStatus, InvoiceWriter, PaymentData, PaymentWriter,
    StoreId,
};
use uuid::Uuid;

use super::helpers::{RecordingPaymentObserver, create_test_consumer_with_observer};

const ONE_ETH: &str = "1000000000000000000";

/// An invoice on `store_id` in `status`, already carrying its full amount, and
/// the confirmed payment that satisfies it.
async fn settle_one(
    ds: &Arc<InMemoryDataService>,
    store_id: StoreId,
    status: InvoiceStatus,
    metadata: Option<serde_json::Value>,
) -> (InvoiceId, PaymentConfirmed) {
    let invoice_id = InvoiceId::new();

    let invoice = InvoiceData {
        id: invoice_id.clone(),
        store_id,
        currency: "ETH".to_string(),
        status,
        amount: ONE_ETH.to_string(),
        amount_received: ONE_ETH.to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata,
        customer_email: None,
        extra: None,
    };
    InvoiceWriter::upsert(&**ds, &invoice).await.unwrap();

    let tx_hash = B256::repeat_byte(0xcd);
    let payment = PaymentData {
        id: Uuid::new_v4(),
        invoice_id: invoice_id.clone(),
        payment_option_id: None,
        chain_id: ChainId::parse("eip155:1").unwrap(),
        asset_type: types::AssetType::Native,
        amount: ONE_ETH.to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        tx_hash: format!("{tx_hash:#x}"),
        block_number: Some(12_345_678),
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: None,
        reorged: false,
        extra: None,
        credited_amount: Some("1".to_string()),
        rate_used: None,
        rate_applied_at: None,
    };
    PaymentWriter::upsert(&**ds, &payment).await.unwrap();

    let event = PaymentConfirmed {
        tx_index: 0,
        chain_id: 1,
        invoice_id: Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: Address::ZERO,
        amount: U256::from(1_000_000_000_000_000_000_u64),
        tx_hash,
        block_number: 12_345_678,
        confirmations: 12,
        confirmed_at: Utc::now(),
    };

    (invoice_id, event)
}

/// The loop this whole capability exists to close: an invoice the billing
/// plugin issued on our own store is paid, and the plugin is told.
#[tokio::test]
async fn a_settled_invoice_on_our_own_store_reaches_the_plugin() {
    let ds = Arc::new(InMemoryDataService::new());
    let own_store = StoreId::new();
    let observer = Arc::new(RecordingPaymentObserver::new());
    let consumer = create_test_consumer_with_observer(
        ds.clone(),
        Arc::new(MemoryBridge::new()),
        own_store,
        observer.clone(),
    );

    let (invoice_id, event) = settle_one(
        &ds,
        own_store,
        InvoiceStatus::Processing,
        Some(serde_json::json!({ "subscription": "acct-7" })),
    )
    .await;

    consumer.handle_payment_confirmed(event).await.unwrap();

    let settled = observer.settled();
    assert_eq!(
        settled.len(),
        1,
        "the plugin was never told its invoice was paid"
    );
    assert_eq!(settled[0].invoice_id, invoice_id);
    assert_eq!(
        settled[0].status,
        InvoiceStatus::Paid,
        "the observer must see the status that was committed, not the one before the transition"
    );
    assert_eq!(
        settled[0].metadata,
        Some(serde_json::json!({ "subscription": "acct-7" })),
        "the plugin's own metadata is how it knows which subscription this was"
    );
}

/// The disclosure the own-store gate exists to prevent, through the real
/// handler rather than the dispatcher in isolation.
///
/// On its own this cannot tell a working gate from a dispatch that was never
/// wired — an unwired feature leaks nothing either. It is
/// `payment_observer::a_merchants_payment_is_never_reported` that fails when
/// the gate itself is removed; the pair is what pins both properties.
#[tokio::test]
async fn a_settled_invoice_on_a_merchants_store_does_not() {
    let ds = Arc::new(InMemoryDataService::new());
    let own_store = StoreId::new();
    let a_merchant = StoreId::new();
    let observer = Arc::new(RecordingPaymentObserver::new());
    let consumer = create_test_consumer_with_observer(
        ds.clone(),
        Arc::new(MemoryBridge::new()),
        own_store,
        observer.clone(),
    );

    let (_, event) = settle_one(&ds, a_merchant, InvoiceStatus::Processing, None).await;

    consumer.handle_payment_confirmed(event).await.unwrap();

    assert!(
        observer.settled().is_empty(),
        "a plugin must never learn what a merchant was paid"
    );
}

/// The second dispatch site. A subscription paid after its invoice expired has
/// still been paid, and this branch is easy to wire the first one without.
#[tokio::test]
async fn a_late_payment_on_our_own_store_also_reaches_the_plugin() {
    let ds = Arc::new(InMemoryDataService::new());
    let own_store = StoreId::new();
    let observer = Arc::new(RecordingPaymentObserver::new());
    let consumer = create_test_consumer_with_observer(
        ds.clone(),
        Arc::new(MemoryBridge::new()),
        own_store,
        observer.clone(),
    );

    let (_, event) = settle_one(&ds, own_store, InvoiceStatus::Expired, None).await;

    consumer.handle_payment_confirmed(event).await.unwrap();

    let settled = observer.settled();
    assert_eq!(
        settled.len(),
        1,
        "a late payment is money received; the subscription must still be credited"
    );
    assert_eq!(settled[0].status, InvoiceStatus::LatePaid);
}

/// An instance with no billing plugin configured registers no observers, and
/// the handler must not change behaviour because of it.
#[tokio::test]
async fn an_instance_with_no_observers_still_settles_the_invoice() {
    use types::InvoiceReader;

    let ds = Arc::new(InMemoryDataService::new());
    let store_id = StoreId::new();
    let consumer = super::helpers::create_test_consumer(ds.clone(), Arc::new(MemoryBridge::new()));

    let (invoice_id, event) = settle_one(&ds, store_id, InvoiceStatus::Processing, None).await;

    consumer.handle_payment_confirmed(event).await.unwrap();

    let invoice = InvoiceReader::get(&*ds, &invoice_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(invoice.status, InvoiceStatus::Paid);
}
