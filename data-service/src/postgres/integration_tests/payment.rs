//! Payment integration tests.

use chrono::Utc;
use types::{InvoiceWriter, PaymentReader, PaymentWriter};

use super::{create_test_service, test_invoice, test_payment};

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
