//! Integration tests for PostgreSQL repository implementations.
//! Require DATABASE_URL environment variable.
//! Run with: DATABASE_URL="postgres://..." cargo test -p data-service -- --ignored

use chrono::{Duration, Utc};

use types::{
    InvoiceQueryParams, InvoiceReader, InvoiceStatus, InvoiceWriter, Network, PaymentReader,
    PaymentWriter, WatchedAddressReader, WatchedAddressWriter,
};

use super::tests::{create_test_service, test_invoice, test_payment, unique_address};

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
    assert_eq!(fetched.network, Network::Ethereum);
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
    invoice1.network = Network::Ethereum;
    InvoiceWriter::upsert(&service, &invoice1).await.unwrap();

    let mut invoice2 = test_invoice();
    invoice2.status = InvoiceStatus::Paid;
    invoice2.network = Network::Ethereum;
    InvoiceWriter::upsert(&service, &invoice2).await.unwrap();

    let mut invoice3 = test_invoice();
    invoice3.status = InvoiceStatus::Pending;
    invoice3.network = Network::Polygon;
    InvoiceWriter::upsert(&service, &invoice3).await.unwrap();

    // Query by status
    let params = InvoiceQueryParams::new().with_status(InvoiceStatus::Pending);
    let (total, invoices) = InvoiceReader::query(&service, &params).await.unwrap();
    assert!(total >= 2);
    assert!(invoices.iter().all(|i| i.status == InvoiceStatus::Pending));

    // Query by network
    let params = InvoiceQueryParams::new().with_network(Network::Polygon);
    let (total, invoices) = InvoiceReader::query(&service, &params).await.unwrap();
    assert!(total >= 1);
    assert!(invoices.iter().all(|i| i.network == Network::Polygon));

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
        "chain_id": 1
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
    assert_eq!(fetched.confirmations, 0);
    assert!(fetched.confirmed_at.is_none());

    // Update confirmations
    let confirmed_at = Utc::now();
    PaymentWriter::update_confirmations(&service, payment.id, 12, Some(confirmed_at))
        .await
        .unwrap();

    let fetched = PaymentReader::get(&service, payment.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.confirmations, 12);
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
async fn integration_payment_get_unconfirmed() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create invoice
    let invoice = test_invoice();
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // Create unconfirmed payment
    let mut unconfirmed = test_payment(&invoice.id);
    unconfirmed.confirmations = 2;
    PaymentWriter::upsert(&service, &unconfirmed).await.unwrap();

    // Create confirmed payment
    let mut confirmed = test_payment(&invoice.id);
    confirmed.confirmations = 15;
    PaymentWriter::upsert(&service, &confirmed).await.unwrap();

    // Get unconfirmed (min 12 confirmations)
    let unconfirmed_payments = PaymentReader::get_unconfirmed(&service, 12).await.unwrap();

    // Should include our unconfirmed payment
    assert!(unconfirmed_payments.iter().any(|p| p.id == unconfirmed.id));
    // Should not include our confirmed payment
    assert!(!unconfirmed_payments.iter().any(|p| p.id == confirmed.id));
}

#[tokio::test]
#[ignore]
async fn integration_payment_upsert_update() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create invoice
    let invoice = test_invoice();
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // Create payment
    let mut payment = test_payment(&invoice.id);
    payment.confirmations = 0;
    payment.block_number = None;
    PaymentWriter::upsert(&service, &payment).await.unwrap();

    // Upsert with updated data
    payment.confirmations = 6;
    payment.block_number = Some(12345680);
    payment.confirmed_at = Some(Utc::now());
    PaymentWriter::upsert(&service, &payment).await.unwrap();

    // Verify update
    let fetched = PaymentReader::get(&service, payment.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.confirmations, 6);
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

    // Create invoice first (watched_addresses have FK to invoices)
    let invoice = test_invoice();
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    let address = unique_address();
    let network = Network::Ethereum;

    // Upsert watched address
    WatchedAddressWriter::upsert(&service, &address, &invoice.id, network)
        .await
        .unwrap();

    // Get invoice_id by address
    let found = WatchedAddressReader::get_invoice_id(&service, &address, network)
        .await
        .unwrap();
    assert_eq!(found, Some(invoice.id.clone()));

    // Remove watched address
    WatchedAddressWriter::remove(&service, &address, network)
        .await
        .unwrap();

    // Should not find it anymore
    let found = WatchedAddressReader::get_invoice_id(&service, &address, network)
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

    let address = unique_address();
    let network = Network::Ethereum;

    // Add watched address
    WatchedAddressWriter::upsert(&service, &address, &invoice.id, network)
        .await
        .unwrap();

    // Get active addresses
    let active = WatchedAddressReader::get_active(&service).await.unwrap();

    // Should include our address
    assert!(active
        .iter()
        .any(|(a, id, n)| a == &address && id == &invoice.id && *n == network));

    // Remove it
    WatchedAddressWriter::remove(&service, &address, network)
        .await
        .unwrap();

    // Should not be in active anymore
    let active = WatchedAddressReader::get_active(&service).await.unwrap();
    assert!(!active.iter().any(|(a, _, _)| a == &address));
}

#[tokio::test]
#[ignore]
async fn integration_watched_address_different_networks() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create two invoices
    let invoice1 = test_invoice();
    let mut invoice2 = test_invoice();
    invoice2.network = Network::Polygon;
    InvoiceWriter::upsert(&service, &invoice1).await.unwrap();
    InvoiceWriter::upsert(&service, &invoice2).await.unwrap();

    let address = unique_address();

    // Watch same address on different networks
    WatchedAddressWriter::upsert(&service, &address, &invoice1.id, Network::Ethereum)
        .await
        .unwrap();
    WatchedAddressWriter::upsert(&service, &address, &invoice2.id, Network::Polygon)
        .await
        .unwrap();

    // Each network should return the correct invoice
    let found_eth = WatchedAddressReader::get_invoice_id(&service, &address, Network::Ethereum)
        .await
        .unwrap();
    assert_eq!(found_eth, Some(invoice1.id.clone()));

    let found_polygon = WatchedAddressReader::get_invoice_id(&service, &address, Network::Polygon)
        .await
        .unwrap();
    assert_eq!(found_polygon, Some(invoice2.id.clone()));

    // Remove from one network shouldn't affect the other
    WatchedAddressWriter::remove(&service, &address, Network::Ethereum)
        .await
        .unwrap();

    let found_eth = WatchedAddressReader::get_invoice_id(&service, &address, Network::Ethereum)
        .await
        .unwrap();
    assert!(found_eth.is_none());

    let found_polygon = WatchedAddressReader::get_invoice_id(&service, &address, Network::Polygon)
        .await
        .unwrap();
    assert_eq!(found_polygon, Some(invoice2.id.clone()));
}

#[tokio::test]
#[ignore]
async fn integration_watched_address_upsert_replaces() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create two invoices
    let invoice1 = test_invoice();
    let invoice2 = test_invoice();
    InvoiceWriter::upsert(&service, &invoice1).await.unwrap();
    InvoiceWriter::upsert(&service, &invoice2).await.unwrap();

    let address = unique_address();
    let network = Network::Ethereum;

    // Watch address for invoice1
    WatchedAddressWriter::upsert(&service, &address, &invoice1.id, network)
        .await
        .unwrap();

    let found = WatchedAddressReader::get_invoice_id(&service, &address, network)
        .await
        .unwrap();
    assert_eq!(found, Some(invoice1.id.clone()));

    // Upsert same address for invoice2 - should replace
    WatchedAddressWriter::upsert(&service, &address, &invoice2.id, network)
        .await
        .unwrap();

    let found = WatchedAddressReader::get_invoice_id(&service, &address, network)
        .await
        .unwrap();
    assert_eq!(found, Some(invoice2.id.clone()));
}
