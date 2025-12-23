//! Test helpers for PostgreSQL repository implementations.

use chrono::{Duration, Utc};
use uuid::Uuid;

use types::{InvoiceData, InvoiceId, InvoiceStatus, Network, PaymentData, StoreId};

use super::PgDataService;

// =========================================================================
// Test helpers - shared with integration tests
// =========================================================================

pub(super) async fn create_test_service() -> Option<PgDataService> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .ok()?;
    Some(PgDataService::new(pool))
}

pub(super) fn test_invoice() -> InvoiceData {
    InvoiceData {
        id: InvoiceId::new(),
        store_id: StoreId::new(),
        network: Network::Ethereum,
        status: InvoiceStatus::Pending,
        amount: "1000000000000000000".to_string(), // 1 ETH in wei
        amount_received: "0".to_string(),
        asset_symbol: "ETH".to_string(),
        payment_address: Some("0x1234567890abcdef1234567890abcdef12345678".to_string()),
        payment_request: None,
        created_at: Utc::now(),
        expires_at: Utc::now() + Duration::hours(1),
        metadata: None,
        extra: None,
    }
}

pub(super) fn test_payment(invoice_id: &InvoiceId) -> PaymentData {
    PaymentData {
        id: Uuid::new_v4(),
        invoice_id: invoice_id.clone(),
        network: Network::Ethereum,
        amount: "1000000000000000000".to_string(),
        asset_symbol: "ETH".to_string(),
        tx_hash: format!("0x{:064x}", Uuid::new_v4().as_u128()),
        block_number: Some(12345678),
        confirmations: 0,
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: Some("0xabcdef1234567890abcdef1234567890abcdef12".to_string()),
        extra: None,
    }
}

pub(super) fn unique_address() -> String {
    format!("0x{:040x}", Uuid::new_v4().as_u128())
}

// =========================================================================
// Unit tests for conversions
// =========================================================================

use super::conversions::{status_to_db, try_db_to_network, try_db_to_status, try_network_to_db};

#[test]
fn test_network_conversion_success() {
    assert_eq!(try_network_to_db(Network::Ethereum).unwrap(), "ethereum");
    assert_eq!(try_network_to_db(Network::Polygon).unwrap(), "polygon");
    assert_eq!(
        try_network_to_db(Network::BinanceSmartChain).unwrap(),
        "binance_smart_chain"
    );

    assert_eq!(try_db_to_network("ethereum").unwrap(), Network::Ethereum);
    assert_eq!(try_db_to_network("polygon").unwrap(), Network::Polygon);
    assert_eq!(
        try_db_to_network("binance_smart_chain").unwrap(),
        Network::BinanceSmartChain
    );
}

#[test]
fn test_network_conversion_errors() {
    // Bitcoin networks should fail
    assert!(try_network_to_db(Network::BitcoinMainnet).is_err());
    assert!(try_network_to_db(Network::BitcoinLightning).is_err());

    // Unknown database values should fail
    assert!(try_db_to_network("bitcoin").is_err());
    assert!(try_db_to_network("unknown_network").is_err());
}

#[test]
fn test_status_conversion() {
    assert_eq!(status_to_db(InvoiceStatus::Pending), "pending");
    assert_eq!(status_to_db(InvoiceStatus::Paid), "paid");
    assert_eq!(status_to_db(InvoiceStatus::PartiallyPaid), "partially_paid");

    assert_eq!(try_db_to_status("pending").unwrap(), InvoiceStatus::Pending);
    assert_eq!(try_db_to_status("paid").unwrap(), InvoiceStatus::Paid);
    assert_eq!(
        try_db_to_status("partially_paid").unwrap(),
        InvoiceStatus::PartiallyPaid
    );
}

#[test]
fn test_status_conversion_error() {
    assert!(try_db_to_status("unknown_status").is_err());
}
