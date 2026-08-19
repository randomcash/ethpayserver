//! Invoice integration tests.

use chrono::{Duration, Utc};
use types::{InvoiceQueryParams, InvoiceReader, InvoiceStatus, InvoiceWriter};

use super::{assert_amount_eq, create_test_service, seeded_test_invoice};

#[tokio::test]
#[ignore]
async fn integration_invoice_crud() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create invoice
    let invoice = seeded_test_invoice(&service).await;
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // Get invoice
    let fetched = InvoiceReader::get(&service, &invoice.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.id, invoice.id);
    assert_eq!(fetched.status, InvoiceStatus::Pending);
    assert_eq!(fetched.currency, "ETH");
    assert_amount_eq(
        &fetched.amount,
        &invoice.amount,
        "invoice amount round-trip",
    );

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
    assert_amount_eq(
        &fetched.amount_received,
        "500000000000000000",
        "partial payment",
    );

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
    assert_amount_eq(
        &fetched.amount_received,
        "1000000000000000000",
        "full payment",
    );
}

#[tokio::test]
#[ignore]
async fn integration_invoice_query() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create multiple invoices
    let mut invoice1 = seeded_test_invoice(&service).await;
    invoice1.status = InvoiceStatus::Pending;
    invoice1.currency = "ETH".to_string();
    InvoiceWriter::upsert(&service, &invoice1).await.unwrap();

    let mut invoice2 = seeded_test_invoice(&service).await;
    invoice2.status = InvoiceStatus::Paid;
    invoice2.currency = "ETH".to_string();
    InvoiceWriter::upsert(&service, &invoice2).await.unwrap();

    let mut invoice3 = seeded_test_invoice(&service).await;
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
    let mut expired_invoice = seeded_test_invoice(&service).await;
    expired_invoice.expires_at = Utc::now() - Duration::hours(1);
    expired_invoice.status = InvoiceStatus::Pending;
    InvoiceWriter::upsert(&service, &expired_invoice)
        .await
        .unwrap();

    // Create a non-expired invoice
    let mut active_invoice = seeded_test_invoice(&service).await;
    active_invoice.expires_at = Utc::now() + Duration::hours(1);
    active_invoice.status = InvoiceStatus::Pending;
    InvoiceWriter::upsert(&service, &active_invoice)
        .await
        .unwrap();

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

    let mut invoice = seeded_test_invoice(&service).await;
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
