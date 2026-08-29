//! Refund repository integration tests.

use chrono::Utc;
use types::{InvoiceWriter, PaymentWriter, RefundData, RefundStatus, StoreId};
use uuid::Uuid;

use crate::{RefundReader, RefundWriter};

use super::super::PgDataService;
use super::{create_test_service, seeded_test_invoice, test_payment};

/// Seed the rows a refund's foreign keys require (`stores`, `payments`) and
/// return a pending refund bound to them.
///
/// `refunds.payment_id` REFERENCES payments(id) and `refunds.store_id`
/// REFERENCES stores(id), so a refund built from bare UUIDs violates both.
async fn seeded_test_refund(service: &PgDataService) -> RefundData {
    let invoice = seeded_test_invoice(service).await;
    InvoiceWriter::upsert(service, &invoice).await.unwrap();

    let payment = test_payment(&invoice.id);
    PaymentWriter::upsert(service, &payment).await.unwrap();

    RefundData {
        id: Uuid::new_v4(),
        invoice_id: invoice.id.clone(),
        payment_id: payment.id,
        store_id: invoice.store_id,
        to_address: payment
            .from_address
            .clone()
            .expect("test payment has a from_address"),
        chain_id: payment.chain_id,
        asset_type: "native".to_string(),
        asset_symbol: payment.asset_symbol.clone(),
        token_address: None,
        amount: payment.amount.clone(),
        tx_hash: None,
        status: RefundStatus::Pending,
        fee_amount: None,
        reason: Some("customer request".to_string()),
        error_message: None,
        created_at: Utc::now(),
        confirmed_at: None,
    }
}

#[tokio::test]
#[ignore]
async fn integration_refund_create_and_read() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let refund = seeded_test_refund(&service).await;
    RefundWriter::create_refund(&service, &refund).await.unwrap();

    let fetched = RefundReader::get_refund(&service, refund.id)
        .await
        .unwrap()
        .expect("refund was just created");

    assert_eq!(fetched.id, refund.id);
    assert_eq!(fetched.invoice_id, refund.invoice_id);
    assert_eq!(fetched.payment_id, refund.payment_id);
    assert_eq!(fetched.store_id, refund.store_id);
    assert_eq!(fetched.to_address, refund.to_address);
    assert_eq!(fetched.chain_id, refund.chain_id);
    assert_eq!(fetched.asset_type, "native");
    assert_eq!(fetched.asset_symbol, refund.asset_symbol);
    assert_eq!(fetched.amount, refund.amount);
    assert_eq!(fetched.status, RefundStatus::Pending);
    assert_eq!(fetched.reason.as_deref(), Some("customer request"));
    assert!(fetched.tx_hash.is_none());
    assert!(fetched.confirmed_at.is_none());
}

#[tokio::test]
#[ignore]
async fn integration_refund_get_unknown_id_returns_none() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let fetched = RefundReader::get_refund(&service, Uuid::new_v4())
        .await
        .unwrap();

    assert!(fetched.is_none());
}

#[tokio::test]
#[ignore]
async fn integration_refund_update_status_sets_tx_hash_and_fee() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let refund = seeded_test_refund(&service).await;
    RefundWriter::create_refund(&service, &refund).await.unwrap();

    RefundWriter::update_refund_status(
        &service,
        refund.id,
        RefundStatus::Broadcasting,
        Some("0xrefundtx"),
        Some("21000"),
        None,
    )
    .await
    .unwrap();

    let fetched = RefundReader::get_refund(&service, refund.id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(fetched.status, RefundStatus::Broadcasting);
    assert_eq!(fetched.tx_hash.as_deref(), Some("0xrefundtx"));
    assert_eq!(fetched.fee_amount.as_deref(), Some("21000"));
    assert!(!fetched.status.is_final());
}

/// `update_refund_status` COALESCEs tx_hash/fee/error, so a later update that
/// passes `None` must not wipe the hash recorded at broadcast time.
#[tokio::test]
#[ignore]
async fn integration_refund_update_status_preserves_existing_tx_hash() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let refund = seeded_test_refund(&service).await;
    RefundWriter::create_refund(&service, &refund).await.unwrap();

    RefundWriter::update_refund_status(
        &service,
        refund.id,
        RefundStatus::Broadcasting,
        Some("0xrefundtx"),
        None,
        None,
    )
    .await
    .unwrap();

    RefundWriter::update_refund_status(&service, refund.id, RefundStatus::Failed, None, None, Some("out of gas"))
        .await
        .unwrap();

    let fetched = RefundReader::get_refund(&service, refund.id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(fetched.status, RefundStatus::Failed);
    assert_eq!(fetched.tx_hash.as_deref(), Some("0xrefundtx"));
    assert_eq!(fetched.error_message.as_deref(), Some("out of gas"));
}

#[tokio::test]
#[ignore]
async fn integration_refund_confirm_sets_confirmed_at() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let refund = seeded_test_refund(&service).await;
    RefundWriter::create_refund(&service, &refund).await.unwrap();

    RefundWriter::confirm_refund(&service, refund.id)
        .await
        .unwrap();

    let fetched = RefundReader::get_refund(&service, refund.id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(fetched.status, RefundStatus::Confirmed);
    assert!(fetched.status.is_final());
    assert!(fetched.confirmed_at.is_some());
}

#[tokio::test]
#[ignore]
async fn integration_refund_get_for_invoice() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let first = seeded_test_refund(&service).await;
    RefundWriter::create_refund(&service, &first).await.unwrap();

    // A second partial refund against the same invoice and payment.
    let second = RefundData {
        id: Uuid::new_v4(),
        amount: "500000000000000000".to_string(),
        ..first.clone()
    };
    RefundWriter::create_refund(&service, &second).await.unwrap();

    let refunds = RefundReader::get_refunds_for_invoice(&service, &first.invoice_id)
        .await
        .unwrap();

    assert_eq!(refunds.len(), 2);
    assert!(refunds.iter().all(|r| r.invoice_id == first.invoice_id));

    let ids: Vec<Uuid> = refunds.iter().map(|r| r.id).collect();
    assert!(ids.contains(&first.id));
    assert!(ids.contains(&second.id));
}

#[tokio::test]
#[ignore]
async fn integration_refund_get_for_invoice_empty() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let invoice = seeded_test_invoice(&service).await;
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    let refunds = RefundReader::get_refunds_for_invoice(&service, &invoice.id)
        .await
        .unwrap();

    assert!(refunds.is_empty());
}

#[tokio::test]
#[ignore]
async fn integration_refund_get_for_store_is_scoped_and_counted() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let refund = seeded_test_refund(&service).await;
    RefundWriter::create_refund(&service, &refund).await.unwrap();

    let (total, refunds) = RefundReader::get_refunds_for_store(&service, refund.store_id, 50, 0)
        .await
        .unwrap();

    assert_eq!(total, 1);
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].id, refund.id);

    // A store with no refunds must not see another store's rows.
    let (other_total, other_refunds) =
        RefundReader::get_refunds_for_store(&service, StoreId::new(), 50, 0)
            .await
            .unwrap();

    assert_eq!(other_total, 0);
    assert!(other_refunds.is_empty());
}

#[tokio::test]
#[ignore]
async fn integration_refund_get_for_store_paginates() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let first = seeded_test_refund(&service).await;
    RefundWriter::create_refund(&service, &first).await.unwrap();
    let second = RefundData {
        id: Uuid::new_v4(),
        ..first.clone()
    };
    RefundWriter::create_refund(&service, &second).await.unwrap();

    let (total, page) = RefundReader::get_refunds_for_store(&service, first.store_id, 1, 0)
        .await
        .unwrap();

    // The count reflects every refund for the store, the page only the limit.
    assert_eq!(total, 2);
    assert_eq!(page.len(), 1);

    let (_, second_page) = RefundReader::get_refunds_for_store(&service, first.store_id, 1, 1)
        .await
        .unwrap();

    assert_eq!(second_page.len(), 1);
    assert_ne!(second_page[0].id, page[0].id);
}

/// The refund broadcaster picks up work via `get_active_refunds`, so a refund
/// must appear there while pending and drop out once it reaches a final state.
#[tokio::test]
#[ignore]
async fn integration_refund_active_excludes_final_statuses() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let refund = seeded_test_refund(&service).await;
    RefundWriter::create_refund(&service, &refund).await.unwrap();

    let active = RefundReader::get_active_refunds(&service).await.unwrap();
    assert!(active.iter().any(|r| r.id == refund.id));

    RefundWriter::confirm_refund(&service, refund.id)
        .await
        .unwrap();

    let active = RefundReader::get_active_refunds(&service).await.unwrap();
    assert!(!active.iter().any(|r| r.id == refund.id));
}
