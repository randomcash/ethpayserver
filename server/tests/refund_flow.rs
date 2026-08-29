#![allow(clippy::unwrap_used, clippy::expect_used)]
//! End-to-end integration test for the refund flow.
//!
//! Verifies the full lifecycle: paid invoice → refund eligibility check →
//! payment selection → refund record created pending → picked up by the
//! broadcaster → broadcast → confirmed on-chain → invoice marked refunded.
//!
//! Uses `InMemoryDataService` so the flow runs without a database, and drives
//! the eligibility rules through the same `server::api::refunds` helpers the
//! `create_refund` handler uses — the handler itself takes a `PgAppState`,
//! which is bound to a live Postgres pool and so cannot be called here.

use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use data_service::InMemoryDataService;
use server::api::refunds::{is_refundable_status, resolve_refund_amount, select_refundable_payment};
use types::{
    AssetType, InvoiceData, InvoiceId, InvoiceReader, InvoiceStatus, InvoiceWriter, PaymentData,
    PaymentReader, PaymentWriter, RefundData, RefundReader, RefundStatus, RefundWriter, StoreId,
};

/// Sepolia chain ID used in tests.
const TEST_CHAIN_ID: u64 = 11155111;
const PAYER: &str = "0xabcdef1234567890abcdef1234567890abcdef12";
const ONE_ETH_WEI: &str = "1000000000000000000";

fn test_invoice(store_id: StoreId) -> InvoiceData {
    InvoiceData {
        id: InvoiceId::new(),
        store_id,
        currency: "USD".to_string(),
        status: InvoiceStatus::Pending,
        amount: "100.00".to_string(),
        amount_received: "0".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        extra: None,
    }
}

fn test_payment(invoice_id: &InvoiceId) -> PaymentData {
    PaymentData {
        id: Uuid::new_v4(),
        invoice_id: invoice_id.clone(),
        payment_option_id: None,
        chain_id: TEST_CHAIN_ID,
        asset_type: AssetType::Native,
        amount: ONE_ETH_WEI.to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        tx_hash: format!("0x{:064x}", Uuid::new_v4().as_u128()),
        block_number: Some(12_345_678),
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: Some(PAYER.to_string()),
        reorged: false,
        extra: None,
        credited_amount: None,
        rate_used: None,
        rate_applied_at: None,
    }
}

/// Drive an invoice to Paid with one confirmed payment, the state a refund
/// starts from.
async fn paid_invoice_with_confirmed_payment(
    ds: &InMemoryDataService,
) -> (InvoiceData, PaymentData) {
    let invoice = test_invoice(StoreId::new());
    InvoiceWriter::upsert(ds, &invoice).await.unwrap();

    let payment = test_payment(&invoice.id);
    PaymentWriter::upsert(ds, &payment).await.unwrap();
    PaymentWriter::mark_confirmed(ds, payment.id, Utc::now())
        .await
        .unwrap();

    InvoiceWriter::update_amount_received(ds, &invoice.id, ONE_ETH_WEI)
        .await
        .unwrap();
    InvoiceWriter::update_status(ds, &invoice.id, InvoiceStatus::Paid)
        .await
        .unwrap();

    let invoice = InvoiceReader::get(ds, &invoice.id)
        .await
        .unwrap()
        .expect("invoice was just written");
    let payment = PaymentReader::get(ds, payment.id)
        .await
        .unwrap()
        .expect("payment was just written");

    (invoice, payment)
}

/// Build the refund record `create_refund` would write for `payment`.
fn refund_for(invoice: &InvoiceData, payment: &PaymentData, amount: String) -> RefundData {
    RefundData {
        id: Uuid::new_v4(),
        invoice_id: invoice.id.clone(),
        payment_id: payment.id,
        store_id: invoice.store_id,
        to_address: payment
            .from_address
            .clone()
            .expect("a refundable payment has a from_address"),
        chain_id: payment.chain_id,
        asset_type: payment.asset_type.to_string(),
        asset_symbol: payment.asset_symbol.clone(),
        token_address: payment.token_address.clone(),
        amount,
        tx_hash: None,
        status: RefundStatus::Pending,
        fee_amount: None,
        reason: Some("customer request".to_string()),
        error_message: None,
        created_at: Utc::now(),
        confirmed_at: None,
    }
}

/// The happy path, start to finish: a paid invoice is refunded in full, the
/// refund is broadcast and confirmed, and the invoice ends up Refunded.
#[tokio::test]
#[allow(clippy::too_many_lines)] // one end-to-end narrative — splitting it hides the flow
async fn refund_flow_paid_invoice_through_to_confirmed() {
    let ds = Arc::new(InMemoryDataService::new());

    // --- 1. A paid invoice with a confirmed payment ------------------------
    let (invoice, payment) = paid_invoice_with_confirmed_payment(&ds).await;
    assert_eq!(invoice.status, InvoiceStatus::Paid);
    assert!(payment.confirmed_at.is_some());

    // --- 2. The refund request is accepted ---------------------------------
    assert!(
        is_refundable_status(invoice.status),
        "a paid invoice is refundable"
    );

    let payments = PaymentReader::get_valid_for_invoice(&*ds, &invoice.id)
        .await
        .unwrap();
    let selected =
        select_refundable_payment(payments).expect("the confirmed payment is refundable");
    assert_eq!(selected.id, payment.id);

    // No amount given, so the full payment is refunded.
    let amount = resolve_refund_amount(None, &selected.amount);
    assert_eq!(amount, ONE_ETH_WEI);

    // --- 3. The refund record is created as pending ------------------------
    let refund = refund_for(&invoice, &selected, amount);
    RefundWriter::create_refund(&*ds, &refund).await.unwrap();

    let stored = RefundReader::get_refund(&*ds, refund.id)
        .await
        .unwrap()
        .expect("refund was just created");
    assert_eq!(stored.status, RefundStatus::Pending);
    assert_eq!(stored.to_address, PAYER, "refunds go back to the payer");
    assert_eq!(stored.amount, ONE_ETH_WEI);
    assert!(stored.tx_hash.is_none());
    assert!(stored.confirmed_at.is_none());

    // It is listed against the invoice.
    let listed = RefundReader::get_refunds_for_invoice(&*ds, &invoice.id)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, refund.id);

    // --- 4. The broadcaster picks it up ------------------------------------
    let active = RefundReader::get_active_refunds(&*ds).await.unwrap();
    assert!(
        active.iter().any(|r| r.id == refund.id),
        "a pending refund is queued for broadcast"
    );

    RefundWriter::update_refund_status(
        &*ds,
        refund.id,
        RefundStatus::Broadcasting,
        Some("0xrefundtx"),
        Some("21000"),
        None,
    )
    .await
    .unwrap();

    let stored = RefundReader::get_refund(&*ds, refund.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, RefundStatus::Broadcasting);
    assert_eq!(stored.tx_hash.as_deref(), Some("0xrefundtx"));
    assert!(!stored.status.is_final(), "still in flight");
    assert!(
        RefundReader::get_active_refunds(&*ds)
            .await
            .unwrap()
            .iter()
            .any(|r| r.id == refund.id),
        "a broadcasting refund is still monitored"
    );

    // --- 5. The transaction confirms ---------------------------------------
    RefundWriter::confirm_refund(&*ds, refund.id).await.unwrap();

    let stored = RefundReader::get_refund(&*ds, refund.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, RefundStatus::Confirmed);
    assert!(stored.status.is_final());
    assert!(stored.confirmed_at.is_some());
    assert_eq!(
        stored.tx_hash.as_deref(),
        Some("0xrefundtx"),
        "confirming must not drop the transaction hash"
    );

    assert!(
        !RefundReader::get_active_refunds(&*ds)
            .await
            .unwrap()
            .iter()
            .any(|r| r.id == refund.id),
        "a confirmed refund leaves the monitoring queue"
    );

    // --- 6. The invoice settles as refunded --------------------------------
    InvoiceWriter::update_status(&*ds, &invoice.id, InvoiceStatus::Refunded)
        .await
        .unwrap();

    let invoice = InvoiceReader::get(&*ds, &invoice.id).await.unwrap().unwrap();
    assert_eq!(invoice.status, InvoiceStatus::Refunded);
    assert!(
        !is_refundable_status(invoice.status),
        "an already-refunded invoice cannot be refunded again"
    );
}

/// A refund that fails on-chain keeps its transaction hash and records why —
/// `update_refund_status` COALESCEs, so the later `None` must not clear it.
#[tokio::test]
async fn refund_flow_failed_broadcast_records_the_error() {
    let ds = Arc::new(InMemoryDataService::new());
    let (invoice, payment) = paid_invoice_with_confirmed_payment(&ds).await;

    let refund = refund_for(&invoice, &payment, ONE_ETH_WEI.to_string());
    RefundWriter::create_refund(&*ds, &refund).await.unwrap();

    RefundWriter::update_refund_status(
        &*ds,
        refund.id,
        RefundStatus::Broadcasting,
        Some("0xrefundtx"),
        None,
        None,
    )
    .await
    .unwrap();

    RefundWriter::update_refund_status(
        &*ds,
        refund.id,
        RefundStatus::Failed,
        None,
        None,
        Some("insufficient balance"),
    )
    .await
    .unwrap();

    let stored = RefundReader::get_refund(&*ds, refund.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, RefundStatus::Failed);
    assert!(stored.status.is_final());
    assert_eq!(stored.tx_hash.as_deref(), Some("0xrefundtx"));
    assert_eq!(stored.error_message.as_deref(), Some("insufficient balance"));
    assert!(stored.confirmed_at.is_none());

    assert!(
        !RefundReader::get_active_refunds(&*ds)
            .await
            .unwrap()
            .iter()
            .any(|r| r.id == refund.id),
        "a failed refund leaves the monitoring queue"
    );
}

/// Two partial refunds against one payment both settle and both stay listed
/// against the invoice.
#[tokio::test]
async fn refund_flow_partial_refunds_accumulate_on_the_invoice() {
    let ds = Arc::new(InMemoryDataService::new());
    let (invoice, payment) = paid_invoice_with_confirmed_payment(&ds).await;

    let half = resolve_refund_amount(Some("500000000000000000".to_string()), &payment.amount);
    assert_eq!(half, "500000000000000000");

    let first = refund_for(&invoice, &payment, half.clone());
    RefundWriter::create_refund(&*ds, &first).await.unwrap();
    let second = refund_for(&invoice, &payment, half);
    RefundWriter::create_refund(&*ds, &second).await.unwrap();

    for id in [first.id, second.id] {
        RefundWriter::confirm_refund(&*ds, id).await.unwrap();
    }

    let listed = RefundReader::get_refunds_for_invoice(&*ds, &invoice.id)
        .await
        .unwrap();
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().all(|r| r.status == RefundStatus::Confirmed));
    assert!(
        listed
            .iter()
            .all(|r| r.amount == "500000000000000000" && r.payment_id == payment.id)
    );

    let (total, page) = RefundReader::get_refunds_for_store(&*ds, invoice.store_id, 50, 0)
        .await
        .unwrap();
    assert_eq!(total, 2);
    assert_eq!(page.len(), 2);
}

/// An invoice that was never paid must be rejected before any refund record is
/// written — otherwise the server would pay out funds it never received.
#[tokio::test]
async fn refund_flow_rejects_an_unpaid_invoice() {
    let ds = Arc::new(InMemoryDataService::new());

    let invoice = test_invoice(StoreId::new());
    InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

    assert!(
        !is_refundable_status(invoice.status),
        "a pending invoice is not refundable"
    );

    let refunds = RefundReader::get_refunds_for_invoice(&*ds, &invoice.id)
        .await
        .unwrap();
    assert!(refunds.is_empty(), "no refund record is created");
}

/// A paid invoice whose only payment was rolled back by a reorg has no
/// refundable payment, so the flow stops before creating a refund.
#[tokio::test]
async fn refund_flow_rejects_a_reorged_payment() {
    let ds = Arc::new(InMemoryDataService::new());
    let (invoice, payment) = paid_invoice_with_confirmed_payment(&ds).await;

    let reorged = PaymentData {
        reorged: true,
        ..payment
    };
    PaymentWriter::upsert(&*ds, &reorged).await.unwrap();

    assert!(is_refundable_status(invoice.status));

    let payments = PaymentReader::get_valid_for_invoice(&*ds, &invoice.id)
        .await
        .unwrap();
    assert!(
        select_refundable_payment(payments).is_none(),
        "a reorged payment must not be refunded"
    );

    let refunds = RefundReader::get_refunds_for_invoice(&*ds, &invoice.id)
        .await
        .unwrap();
    assert!(refunds.is_empty(), "no refund record is created");
}
