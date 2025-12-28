//! Integration tests for PostgreSQL repository implementations.
//! Require DATABASE_URL environment variable.
//! Run with: DATABASE_URL="postgres://..." cargo test -p data-service -- --ignored

use chrono::{Duration, Utc};

use types::{
    InvoiceQueryParams, InvoiceReader, InvoiceStatus, InvoiceWriter, PaymentReader,
    PaymentWriter, PaymentMethodId, PaymentOptionId, PaymentOptionWriter, PaymentOptionData,
    WatchedAddressReader, WatchedAddressWriter,
};

use super::tests::{create_test_service, test_invoice, test_payment, unique_address};

// Helper to create a test payment option
fn test_payment_option(invoice_id: &types::InvoiceId, chain_id: u64) -> PaymentOptionData {
    PaymentOptionData {
        id: PaymentOptionId::new(),
        invoice_id: invoice_id.clone(),
        payment_method_id: PaymentMethodId::new("ETH", chain_id),
        chain_id,
        asset_symbol: "ETH".to_string(),
        token_address: None,
        decimals: 18,
        payment_address: format!("0x{:040x}", uuid::Uuid::new_v4().as_u128()),
        amount: "1000000000000000000".to_string(),
        rate: None,
        rate_at: None,
        is_active: true,
        created_at: Utc::now(),
    }
}

// =========================================================================
// Invoice integration tests
// =========================================================================

#[tokio::test]
#[ignore]
async fn integration_invoice_crud() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create invoice
    let invoice = test_invoice();
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // Get invoice
    let fetched = InvoiceReader::get(&service, &invoice.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.id, invoice.id);
    assert_eq!(fetched.status, InvoiceStatus::Pending);
    assert_eq!(fetched.currency, "ETH");
    assert_eq!(fetched.amount, invoice.amount);

    // Update status
    InvoiceWriter::update_status(&service, &invoice.id, InvoiceStatus::Processing)
        .await
        .unwrap();
    let fetched = InvoiceReader::get(&service, &invoice.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.status, InvoiceStatus::Processing);

    // Update amount received
    InvoiceWriter::update_amount_received(&service, &invoice.id, "500000000000000000")
        .await
        .unwrap();
    let fetched = InvoiceReader::get(&service, &invoice.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.amount_received, "500000000000000000");

    // Upsert (update existing)
    let mut updated = fetched.clone();
    updated.status = InvoiceStatus::Paid;
    updated.amount_received = "1000000000000000000".to_string();
    InvoiceWriter::upsert(&service, &updated).await.unwrap();

    let fetched = InvoiceReader::get(&service, &invoice.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.status, InvoiceStatus::Paid);
    assert_eq!(fetched.amount_received, "1000000000000000000");
}

#[tokio::test]
#[ignore]
async fn integration_invoice_query() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create multiple invoices
    let mut invoice1 = test_invoice();
    invoice1.status = InvoiceStatus::Pending;
    invoice1.currency = "ETH".to_string();
    InvoiceWriter::upsert(&service, &invoice1).await.unwrap();

    let mut invoice2 = test_invoice();
    invoice2.status = InvoiceStatus::Paid;
    invoice2.currency = "ETH".to_string();
    InvoiceWriter::upsert(&service, &invoice2).await.unwrap();

    let mut invoice3 = test_invoice();
    invoice3.status = InvoiceStatus::Pending;
    invoice3.currency = "USDC".to_string();
    InvoiceWriter::upsert(&service, &invoice3).await.unwrap();

    // Query by status
    let params = InvoiceQueryParams::new().with_status(InvoiceStatus::Pending);
    let (total, invoices) = InvoiceReader::query(&service, &params).await.unwrap();
    assert!(total >= 2);
    assert!(invoices.iter().all(|i| i.status == InvoiceStatus::Pending));

    // Query by currency
    let params = InvoiceQueryParams::new().with_currency("USDC");
    let (total, invoices) = InvoiceReader::query(&service, &params).await.unwrap();
    assert!(total >= 1);
    assert!(invoices.iter().all(|i| i.currency == "USDC"));

    // Query with pagination
    let params = InvoiceQueryParams::new().with_limit(1).with_offset(0);
    let (_, invoices) = InvoiceReader::query(&service, &params).await.unwrap();
    assert_eq!(invoices.len(), 1);
}

#[tokio::test]
#[ignore]
async fn integration_invoice_expired() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create an expired invoice
    let mut expired_invoice = test_invoice();
    expired_invoice.expires_at = Utc::now() - Duration::hours(1);
    expired_invoice.status = InvoiceStatus::Pending;
    InvoiceWriter::upsert(&service, &expired_invoice).await.unwrap();

    // Create a non-expired invoice
    let mut active_invoice = test_invoice();
    active_invoice.expires_at = Utc::now() + Duration::hours(1);
    active_invoice.status = InvoiceStatus::Pending;
    InvoiceWriter::upsert(&service, &active_invoice).await.unwrap();

    // Get expired invoices
    let expired = InvoiceReader::get_expired(&service).await.unwrap();

    // Should include our expired invoice
    assert!(expired.iter().any(|i| i.id == expired_invoice.id));
    // Should not include our active invoice
    assert!(!expired.iter().any(|i| i.id == active_invoice.id));
}

#[tokio::test]
#[ignore]
async fn integration_invoice_with_metadata() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let mut invoice = test_invoice();
    invoice.metadata = Some(serde_json::json!({
        "order_id": "12345",
        "customer": "test@example.com"
    }));
    invoice.extra = Some(serde_json::json!({
        "custom_field": "value"
    }));

    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    let fetched = InvoiceReader::get(&service, &invoice.id)
        .await
        .unwrap()
        .unwrap();
    assert!(fetched.metadata.is_some());
    assert!(fetched.extra.is_some());

    let metadata = fetched.metadata.unwrap();
    assert_eq!(metadata["order_id"], "12345");
}

// =========================================================================
// Payment integration tests
// =========================================================================

#[tokio::test]
#[ignore]
async fn integration_payment_crud() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create invoice first (payments have FK to invoices)
    let invoice = test_invoice();
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // Create payment
    let payment = test_payment(&invoice.id);
    PaymentWriter::upsert(&service, &payment).await.unwrap();

    // Get payment
    let fetched = PaymentReader::get(&service, payment.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.id, payment.id);
    assert_eq!(fetched.invoice_id, invoice.id);
    assert!(fetched.confirmed_at.is_none());

    // Mark as confirmed
    let confirmed_at = Utc::now();
    PaymentWriter::mark_confirmed(&service, payment.id, confirmed_at)
        .await
        .unwrap();

    let fetched = PaymentReader::get(&service, payment.id)
        .await
        .unwrap()
        .unwrap();
    assert!(fetched.confirmed_at.is_some());
}

#[tokio::test]
#[ignore]
async fn integration_payment_get_for_invoice() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create invoice
    let invoice = test_invoice();
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // Create multiple payments for the same invoice
    let payment1 = test_payment(&invoice.id);
    let payment2 = test_payment(&invoice.id);
    PaymentWriter::upsert(&service, &payment1).await.unwrap();
    PaymentWriter::upsert(&service, &payment2).await.unwrap();

    // Get payments for invoice
    let payments = PaymentReader::get_for_invoice(&service, &invoice.id)
        .await
        .unwrap();
    assert!(payments.len() >= 2);
    assert!(payments.iter().all(|p| p.invoice_id == invoice.id));
}

#[tokio::test]
#[ignore]
async fn integration_payment_get_awaiting_confirmation() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create invoice
    let invoice = test_invoice();
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // Create unconfirmed payment (confirmed_at = None)
    let unconfirmed = test_payment(&invoice.id);
    PaymentWriter::upsert(&service, &unconfirmed).await.unwrap();

    // Create confirmed payment (confirmed_at = Some)
    let mut confirmed = test_payment(&invoice.id);
    confirmed.confirmed_at = Some(Utc::now());
    PaymentWriter::upsert(&service, &confirmed).await.unwrap();

    // Get awaiting confirmation
    let awaiting = PaymentReader::get_awaiting_confirmation(&service).await.unwrap();

    // Should include our unconfirmed payment
    assert!(awaiting.iter().any(|p| p.id == unconfirmed.id));
    // Should not include our confirmed payment
    assert!(!awaiting.iter().any(|p| p.id == confirmed.id));
}

#[tokio::test]
#[ignore]
async fn integration_payment_upsert_update() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create invoice
    let invoice = test_invoice();
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // Create payment with no block_number initially
    let mut payment = test_payment(&invoice.id);
    payment.block_number = None;
    PaymentWriter::upsert(&service, &payment).await.unwrap();

    // Upsert with updated data
    payment.block_number = Some(12345680);
    payment.confirmed_at = Some(Utc::now());
    PaymentWriter::upsert(&service, &payment).await.unwrap();

    // Verify update
    let fetched = PaymentReader::get(&service, payment.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.block_number, Some(12345680));
    assert!(fetched.confirmed_at.is_some());
}

// =========================================================================
// Watched address integration tests
// =========================================================================

#[tokio::test]
#[ignore]
async fn integration_watched_address_crud() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create invoice first
    let invoice = test_invoice();
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // Create payment option (watched_addresses link to payment_options now)
    let payment_option = test_payment_option(&invoice.id, 1);
    PaymentOptionWriter::create(&service, &payment_option).await.unwrap();

    let address = unique_address();
    let chain_id = 1u64;

    // Upsert watched address
    WatchedAddressWriter::upsert(&service, &address, &payment_option.id, chain_id, None)
        .await
        .unwrap();

    // Get invoice_id by address
    let found = WatchedAddressReader::get_invoice_id(&service, &address, chain_id, None)
        .await
        .unwrap();
    assert_eq!(found, Some(invoice.id.clone()));

    // Get payment_option_id by address
    let found_opt = WatchedAddressReader::get_payment_option_id(&service, &address, chain_id, None)
        .await
        .unwrap();
    assert_eq!(found_opt, Some(payment_option.id.clone()));

    // Deactivate watched address
    WatchedAddressWriter::deactivate(&service, &address, chain_id, None)
        .await
        .unwrap();

    // Should not find it anymore (deactivated)
    let found = WatchedAddressReader::get_invoice_id(&service, &address, chain_id, None)
        .await
        .unwrap();
    assert!(found.is_none());
}

#[tokio::test]
#[ignore]
async fn integration_watched_address_get_active() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create invoice with future expiration
    let mut invoice = test_invoice();
    invoice.expires_at = Utc::now() + Duration::hours(2);
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // Create payment option
    let payment_option = test_payment_option(&invoice.id, 1);
    PaymentOptionWriter::create(&service, &payment_option).await.unwrap();

    let address = unique_address();
    let chain_id = 1u64;

    // Add watched address
    WatchedAddressWriter::upsert(&service, &address, &payment_option.id, chain_id, None)
        .await
        .unwrap();

    // Get active addresses
    let active = WatchedAddressReader::get_active(&service).await.unwrap();

    // Should include our address
    assert!(active
        .iter()
        .any(|(a, po_id, cid, _)| a == &address && po_id == &payment_option.id && *cid == chain_id));

    // Deactivate it
    WatchedAddressWriter::deactivate(&service, &address, chain_id, None)
        .await
        .unwrap();

    // Should not be in active anymore
    let active = WatchedAddressReader::get_active(&service).await.unwrap();
    assert!(!active.iter().any(|(a, _, _, _)| a == &address));
}

#[tokio::test]
#[ignore]
async fn integration_watched_address_different_chains() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create invoice
    let invoice = test_invoice();
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // Create payment options for different chains
    let po1 = test_payment_option(&invoice.id, 1);
    let po2 = test_payment_option(&invoice.id, 137);
    PaymentOptionWriter::create(&service, &po1).await.unwrap();
    PaymentOptionWriter::create(&service, &po2).await.unwrap();

    let address = unique_address();

    // Watch same address on different chain_ids
    WatchedAddressWriter::upsert(&service, &address, &po1.id, 1, None)
        .await
        .unwrap();
    WatchedAddressWriter::upsert(&service, &address, &po2.id, 137, None)
        .await
        .unwrap();

    // Each chain should return the correct payment option
    let found_eth = WatchedAddressReader::get_payment_option_id(&service, &address, 1, None)
        .await
        .unwrap();
    assert_eq!(found_eth, Some(po1.id.clone()));

    let found_polygon = WatchedAddressReader::get_payment_option_id(&service, &address, 137, None)
        .await
        .unwrap();
    assert_eq!(found_polygon, Some(po2.id.clone()));

    // Remove from one chain shouldn't affect the other
    WatchedAddressWriter::deactivate(&service, &address, 1, None)
        .await
        .unwrap();

    let found_eth = WatchedAddressReader::get_payment_option_id(&service, &address, 1, None)
        .await
        .unwrap();
    assert!(found_eth.is_none());

    let found_polygon = WatchedAddressReader::get_payment_option_id(&service, &address, 137, None)
        .await
        .unwrap();
    assert_eq!(found_polygon, Some(po2.id.clone()));
}

#[tokio::test]
#[ignore]
async fn integration_watched_address_upsert_replaces() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create invoice
    let invoice = test_invoice();
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // Create two payment options
    let po1 = test_payment_option(&invoice.id, 1);
    let po2 = test_payment_option(&invoice.id, 1);
    PaymentOptionWriter::create(&service, &po1).await.unwrap();
    PaymentOptionWriter::create(&service, &po2).await.unwrap();

    let address = unique_address();
    let chain_id = 1u64;

    // Watch address for payment_option1
    WatchedAddressWriter::upsert(&service, &address, &po1.id, chain_id, None)
        .await
        .unwrap();

    let found = WatchedAddressReader::get_payment_option_id(&service, &address, chain_id, None)
        .await
        .unwrap();
    assert_eq!(found, Some(po1.id.clone()));

    // Upsert same address for payment_option2 - should replace
    WatchedAddressWriter::upsert(&service, &address, &po2.id, chain_id, None)
        .await
        .unwrap();

    let found = WatchedAddressReader::get_payment_option_id(&service, &address, chain_id, None)
        .await
        .unwrap();
    assert_eq!(found, Some(po2.id.clone()));
}
