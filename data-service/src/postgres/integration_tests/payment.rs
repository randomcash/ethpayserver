//! Payment integration tests.

use chrono::Utc;
use types::{InvoiceWriter, PaymentQueryParams, PaymentReader, PaymentWriter};

use super::{create_test_service, seeded_test_invoice, test_payment};

#[tokio::test]
#[ignore]
async fn integration_payment_crud() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create invoice first (payments have FK to invoices)
    let invoice = seeded_test_invoice(&service).await;
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
    let invoice = seeded_test_invoice(&service).await;
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
    let invoice = seeded_test_invoice(&service).await;
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // Create unconfirmed payment (confirmed_at = None)
    let unconfirmed = test_payment(&invoice.id);
    PaymentWriter::upsert(&service, &unconfirmed).await.unwrap();

    // Create confirmed payment (confirmed_at = Some)
    let mut confirmed = test_payment(&invoice.id);
    confirmed.confirmed_at = Some(Utc::now());
    PaymentWriter::upsert(&service, &confirmed).await.unwrap();

    // Get awaiting confirmation
    let awaiting = PaymentReader::get_awaiting_confirmation(&service)
        .await
        .unwrap();

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
    let invoice = seeded_test_invoice(&service).await;
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

/// RCS-222, the payments half. Worth its own test rather than trusting the
/// invoice one: payments carry no store_id of their own, so scoping them means
/// joining invoices, and the join is only added when a store filter is present.
/// Adding a second store filter without extending that condition would drop the
/// join and fail to compile the SQL — or worse, quietly filter on the wrong
/// table.
#[tokio::test]
#[ignore]
async fn integration_payment_query_scopes_to_a_set_of_stores() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let mine = seeded_test_invoice(&service).await;
    let theirs = seeded_test_invoice(&service).await;
    for inv in [&mine, &theirs] {
        InvoiceWriter::upsert(&service, inv).await.unwrap();
    }

    let mine_payment = test_payment(&mine.id);
    let theirs_payment = test_payment(&theirs.id);
    for p in [&mine_payment, &theirs_payment] {
        PaymentWriter::upsert(&service, p).await.unwrap();
    }

    let (total, rows) = PaymentReader::query(
        &service,
        &PaymentQueryParams::new().with_store_ids(vec![mine.store_id]),
    )
    .await
    .unwrap();

    let ids: Vec<_> = rows.iter().map(|p| p.id).collect();
    assert!(ids.contains(&mine_payment.id), "own payment must appear");
    assert!(
        !ids.contains(&theirs_payment.id),
        "a payment whose invoice belongs to another store must not appear"
    );
    assert_eq!(
        total, 1,
        "count must carry the same filter as the data query"
    );

    let (empty_total, empty_rows) =
        PaymentReader::query(&service, &PaymentQueryParams::new().with_store_ids(vec![]))
            .await
            .unwrap();
    assert_eq!(empty_total, 0, "no memberships must mean no rows");
    assert!(empty_rows.is_empty(), "no memberships must mean no rows");
}
