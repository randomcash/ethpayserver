//! Watched address integration tests.

use chrono::{Duration, Utc};
use types::{InvoiceWriter, PaymentOptionWriter, WatchedAddressReader, WatchedAddressWriter};

use super::{create_test_service, test_invoice, test_payment_option, unique_address};

#[tokio::test]
#[ignore]
async fn integration_watched_address_crud() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create invoice first
    let invoice = test_invoice();
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // Create payment option (watched_addresses link to payment_options now)
    let payment_option = test_payment_option(&invoice.id, 1);
    PaymentOptionWriter::create(&service, &payment_option)
        .await
        .unwrap();

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
    PaymentOptionWriter::create(&service, &payment_option)
        .await
        .unwrap();

    let address = unique_address();
    let chain_id = 1u64;

    // Add watched address
    WatchedAddressWriter::upsert(&service, &address, &payment_option.id, chain_id, None)
        .await
        .unwrap();

    // Get active addresses
    let active = WatchedAddressReader::get_active(&service).await.unwrap();

    // Should include our address
    assert!(active.iter().any(|(a, po_id, cid, _)| a == &address
        && po_id == &payment_option.id
        && *cid == chain_id));

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
