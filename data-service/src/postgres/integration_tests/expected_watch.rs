//! `expected_watched_addresses`: the view the watch reconciler treats as
//! truth.
//!
//! The case that matters is the second test below - a watch whose invoice
//! has already resolved is exactly the state the ticket this exists for
//! calls out: nothing in this schema flips `watched_addresses.is_active` to
//! `FALSE` just because the invoice's status changed, so `is_active = TRUE`
//! alone cannot tell a live invoice from one a cleanup job has not reached
//! yet. Proving the view excludes it is what makes this a check that can
//! fail, rather than one built on a Postgres invariant (the cascade) that
//! makes an "orphaned" row impossible to produce in the first place.

use chrono::{Duration, Utc};
use types::{ChainId, InvoiceStatus, InvoiceWriter, PaymentOptionWriter, WatchedAddressWriter};

use super::{create_test_service, seeded_test_invoice, test_payment_option, unique_address};

#[tokio::test]
#[ignore]
async fn expected_watched_addresses_includes_a_still_pending_invoices_watch() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let mut invoice = seeded_test_invoice(&service).await;
    invoice.expires_at = Utc::now() + Duration::hours(2);
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    let payment_option = test_payment_option(&invoice.id, &ChainId::evm(1));
    PaymentOptionWriter::create(&service, &payment_option)
        .await
        .unwrap();

    let address = unique_address();
    WatchedAddressWriter::upsert(
        &service,
        &address,
        &payment_option.id,
        &ChainId::evm(1),
        None,
    )
    .await
    .unwrap();

    let expected = service.get_expected_watched_addresses().await.unwrap();
    assert!(
        expected
            .iter()
            .any(|w| w.address == address && w.chain_id == ChainId::evm(1)),
        "a still-pending invoice's watch must appear in the expected set"
    );
}

/// The acceptance criterion: seed the exact state the ticket names - an
/// `is_active = TRUE` watched address whose invoice has already resolved,
/// with no cleanup job having run yet - and confirm the view stops treating
/// it as expected. Without this, the view would be exactly as vacuous as
/// the orphan check the ticket rejects: green because the state it claims to
/// catch cannot be produced.
#[tokio::test]
#[ignore]
async fn expected_watched_addresses_excludes_a_resolved_invoices_watch() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let mut invoice = seeded_test_invoice(&service).await;
    invoice.expires_at = Utc::now() + Duration::hours(2);
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    let payment_option = test_payment_option(&invoice.id, &ChainId::evm(1));
    PaymentOptionWriter::create(&service, &payment_option)
        .await
        .unwrap();

    let address = unique_address();
    WatchedAddressWriter::upsert(
        &service,
        &address,
        &payment_option.id,
        &ChainId::evm(1),
        None,
    )
    .await
    .unwrap();

    // Still watched by construction - the same state `hard_delete_store`'s
    // best-effort unwatch can leave behind if it fails, or a paid/cancelled
    // invoice sits in before the cleanup job reaches it.
    let expected = service.get_expected_watched_addresses().await.unwrap();
    assert!(
        expected.iter().any(|w| w.address == address),
        "sanity check: the watch is expected before the invoice resolves"
    );

    InvoiceWriter::update_status(&service, &invoice.id, InvoiceStatus::Paid)
        .await
        .unwrap();

    let expected = service.get_expected_watched_addresses().await.unwrap();
    assert!(
        !expected.iter().any(|w| w.address == address),
        "a paid invoice's watch must not appear as expected, even though \
         nothing has deactivated the watched_addresses row yet"
    );
}
