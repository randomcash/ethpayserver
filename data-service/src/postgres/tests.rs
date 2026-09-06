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

/// Compare two decimal amount strings by value rather than by formatting.
///
/// Amount columns are `numeric(78,18)`, so Postgres returns `"60"` as
/// `"60.000000000000000000"`. Comparing the raw strings makes a correct
/// round-trip look like a failure (RCS-186). Values reach 37 significant
/// digits (1e18 wei with 18 decimals), past what a fixed-width decimal type
/// holds, so normalise the text instead of parsing.
#[track_caller]
pub(super) fn assert_amount_eq(actual: &str, expected: &str, context: &str) {
    assert_eq!(
        normalize_amount(actual),
        normalize_amount(expected),
        "{context} (actual {actual:?}, expected {expected:?})"
    );
}

/// Strip the trailing zeros a fixed-scale `numeric` column pads onto a value.
fn normalize_amount(raw: &str) -> String {
    match raw.split_once('.') {
        Some((int, frac)) => {
            let frac = frac.trim_end_matches('0');
            if frac.is_empty() {
                int.to_string()
            } else {
                format!("{int}.{frac}")
            }
        }
        None => raw.to_string(),
    }
}

#[cfg(test)]
mod amount_tests {
    use super::normalize_amount;

    #[test]
    fn normalises_fixed_scale_padding() {
        assert_eq!(normalize_amount("60.000000000000000000"), "60");
        assert_eq!(
            normalize_amount("1000000000000000000.000000000000000000"),
            "1000000000000000000"
        );
        assert_eq!(normalize_amount("0.050000000000000000"), "0.05");
        assert_eq!(normalize_amount("100"), "100");
        assert_eq!(
            normalize_amount("0.000000000000000001"),
            "0.000000000000000001"
        );
    }
}

/// Seed a real user + store and return the store id.
///
/// `invoices.store_id` is `REFERENCES stores(id)`, and `stores.owner_id` is
/// `REFERENCES users(id)`, so any test that writes an invoice must seed both
/// first. A bare `StoreId::new()` violates `invoices_store_id_fkey` — which is
/// what silently killed every invoice/payment/watched_address/aggregation
/// integration test (RCS-186).
pub(super) async fn seed_store(service: &PgDataService) -> StoreId {
    let user_id = Uuid::new_v4();
    sqlx::query(
        r"
        INSERT INTO users (id, kdf_params, encrypted_symmetric_key, recovery_verification_hash,
                           kdf_salt_identifier)
        VALUES ($1, '{}'::jsonb, '{}'::jsonb, 'test-hash', 'passkey:' || $1::text)
        ",
    )
    .bind(user_id)
    .execute(service.pool())
    .await
    .expect("seed user");

    let store_id = StoreId::new();
    sqlx::query(
        r"
        INSERT INTO stores (id, name, owner_id)
        VALUES ($1, $2, $3)
        ",
    )
    .bind(store_id.0)
    .bind(format!("test-store-{user_id}"))
    .bind(user_id)
    .execute(service.pool())
    .await
    .expect("seed store");

    store_id
}

/// Build a test invoice bound to a freshly seeded store, so the FK holds.
pub(super) async fn seeded_test_invoice(service: &PgDataService) -> InvoiceData {
    let store_id = seed_store(service).await;
    InvoiceData {
        store_id,
        ..test_invoice()
    }
}

pub(super) fn test_invoice() -> InvoiceData {
    InvoiceData {
        id: InvoiceId::new(),
        store_id: StoreId::new(),
        currency: "ETH".to_string(),
        status: InvoiceStatus::Pending,
        amount: "1000000000000000000".to_string(), // 1 ETH in wei
        amount_received: "0".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + Duration::hours(1),
        metadata: None,
        customer_email: None,
        extra: None,
    }
}

pub(super) fn test_payment(invoice_id: &InvoiceId) -> PaymentData {
    PaymentData {
        id: Uuid::new_v4(),
        invoice_id: invoice_id.clone(),
        payment_option_id: None,
        chain_id: 1,
        asset_type: types::AssetType::Native,
        amount: "1000000000000000000".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        tx_hash: format!("0x{:064x}", Uuid::new_v4().as_u128()),
        block_number: Some(12345678),
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: Some("0xabcdef1234567890abcdef1234567890abcdef12".to_string()),
        reorged: false,
        extra: None,
        credited_amount: None,
        rate_used: None,
        rate_applied_at: None,
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
