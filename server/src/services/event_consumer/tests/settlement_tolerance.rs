#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::Utc;
use data_service::{InMemoryDataService, SettlementToleranceReader, SettlementToleranceWriter};
use evm::monitor::bridge::MemoryBridge;
use evm::monitor::events::PaymentConfirmed;
use evm::{Address, B256, U256};
use std::sync::Arc;
use types::ChainId;
use types::{
    InvoiceData, InvoiceId, InvoiceReader, InvoiceStatus, InvoiceWriter, PaymentData,
    PaymentWriter, StoreId,
};
use uuid::Uuid;

use super::helpers::create_test_consumer;

/// Confirm a payment on a 20-unit invoice that has `received` credited, with an
/// optional store tolerance, and return the resulting status and the store.
async fn confirm_with_received(
    received: &str,
    store_tolerance: Option<&str>,
) -> (Arc<InMemoryDataService>, InvoiceId, InvoiceStatus) {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds.clone(), bridge);

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();
    if let Some(t) = store_tolerance {
        SettlementToleranceWriter::set_settlement_tolerance(&*ds, store_id.0, t)
            .await
            .unwrap();
    }

    let invoice = InvoiceData {
        id: invoice_id.clone(),
        store_id,
        currency: "USD".to_string(),
        status: InvoiceStatus::Processing,
        amount: "20.000000000000000000".to_string(),
        amount_received: received.to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        customer_email: None,
        extra: None,
    };
    InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

    let tx_hash = B256::repeat_byte(0xcd);
    let payment = PaymentData {
        id: Uuid::new_v4(),
        invoice_id: invoice_id.clone(),
        payment_option_id: None,
        chain_id: ChainId::parse("eip155:1").unwrap(),
        asset_type: types::AssetType::Native,
        amount: "7470603176500480".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        tx_hash: format!("{:#x}", tx_hash),
        block_number: Some(1),
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: None,
        reorged: false,
        extra: None,
        credited_amount: Some(received.to_string()),
        rate_used: None,
        rate_applied_at: None,
    };
    PaymentWriter::upsert(&*ds, &payment).await.unwrap();

    let event = PaymentConfirmed {
        tx_index: 0,
        chain_id: 1,
        invoice_id: Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: Address::ZERO,
        amount: U256::from(7470603176500480u64),
        tx_hash,
        block_number: 1,
        confirmations: 12,
        confirmed_at: Utc::now(),
    };
    consumer.handle_payment_confirmed(event).await.unwrap();

    let status = InvoiceReader::get(&*ds, &invoice_id)
        .await
        .unwrap()
        .unwrap()
        .status;
    (ds, invoice_id, status)
}

/// The invoice this was found on: exactly the quoted amount paid, short of 20
/// by 2.68e-14. It has to settle, and the allowance has to be on record.
#[tokio::test]
async fn dust_short_of_the_invoice_settles_and_is_recorded() {
    let (ds, id, status) = confirm_with_received("19.999999999999973228", None).await;
    assert_eq!(status, InvoiceStatus::Paid);

    let allowance = SettlementToleranceReader::get_settlement_allowance(&*ds, &id)
        .await
        .unwrap()
        .expect("a tolerance-settled invoice records its allowance");
    assert_eq!(allowance.source, "default");
    assert_eq!(allowance.shortfall.parse::<f64>().unwrap(), 2.6772e-14);
}

#[tokio::test]
async fn exact_payment_records_no_allowance() {
    let (ds, id, status) = confirm_with_received("20.000000000000000000", None).await;
    assert_eq!(status, InvoiceStatus::Paid);
    assert!(
        SettlementToleranceReader::get_settlement_allowance(&*ds, &id)
            .await
            .unwrap()
            .is_none()
    );
}

/// 0.01 short of 20 is 0.05%: well beyond the default, so it is a real
/// underpayment and must not settle.
#[tokio::test]
async fn real_underpayment_does_not_settle_under_the_default() {
    let (ds, id, status) = confirm_with_received("19.99", None).await;
    assert_ne!(status, InvoiceStatus::Paid);
    assert!(
        SettlementToleranceReader::get_settlement_allowance(&*ds, &id)
            .await
            .unwrap()
            .is_none()
    );
}

/// The same 0.05% shortfall settles once the store accepts up to 0.1%, and the
/// record names the store's setting rather than the default.
#[tokio::test]
async fn store_tolerance_widens_what_settles() {
    let (ds, id, status) = confirm_with_received("19.99", Some("0.1")).await;
    assert_eq!(status, InvoiceStatus::Paid);

    let allowance = SettlementToleranceReader::get_settlement_allowance(&*ds, &id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(allowance.source, "store");
    assert_eq!(allowance.tolerance_percent, "0.1");
}
