//! `expected_watched_addresses`: the view the watch reconciler treats as
//! truth.
//!
//! A watch whose invoice has already resolved is exactly the state the
//! ticket this exists for calls out: nothing in this schema flips
//! `watched_addresses.is_active` to `FALSE` just because the invoice's
//! status changed, so `is_active = TRUE` alone cannot tell a live invoice
//! from one a cleanup job has not reached yet. But resolved does not mean
//! "unwatch immediately" either - `InvoiceCleanupService` deliberately keeps
//! a paid invoice's address watched for a grace period afterward so a reorg
//! can still re-validate it, so the view has to track that window rather
//! than excluding every resolved invoice on sight. The tests below cover
//! both edges: still inside the grace window (must stay expected, or the
//! check false-positives on every payment) and long past it (must stop
//! being expected, or the check is vacuous again).

use chrono::{Duration, Utc};
use types::{
    ChainId, InvoiceStatus, InvoiceWriter, PaymentOptionWriter, PaymentWriter, WatchedAddressWriter,
};

use super::{
    create_test_service, seeded_test_invoice, test_payment, test_payment_option, unique_address,
};

#[tokio::test]
#[ignore]
async fn expected_watched_addresses_includes_a_still_pending_invoices_watch() {
    let service = create_test_service().await;

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

/// A paid invoice inside `InvoiceCleanupService`'s grace period is not the
/// vacuous case - it's the one the reviewer of an earlier pass on this
/// ticket caught: `cleanup_paid_addresses` deliberately leaves a just-paid
/// address watched (default 3600s) so a reorg can still re-validate a
/// relocated-but-still-paid transaction. A view that dropped `paid`
/// invoices from "expected" the instant status flips would report every
/// confirmed payment as a false "stale watch" for the length of that
/// window.
#[tokio::test]
#[ignore]
async fn expected_watched_addresses_includes_a_just_paid_invoices_watch() {
    let service = create_test_service().await;

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

    InvoiceWriter::update_status(&service, &invoice.id, InvoiceStatus::Paid)
        .await
        .unwrap();

    let mut payment = test_payment(&invoice.id);
    payment.confirmed_at = Some(Utc::now() - Duration::minutes(5));
    PaymentWriter::upsert(&service, &payment).await.unwrap();

    let expected = service.get_expected_watched_addresses().await.unwrap();
    assert!(
        expected.iter().any(|w| w.address == address),
        "a paid invoice's watch must still appear as expected while inside \
         the cleanup service's grace period - the monitor is still, \
         correctly, watching it"
    );
}

/// The acceptance criterion: seed the exact state the ticket names - an
/// `is_active = TRUE` watched address whose invoice resolved long enough
/// ago that no legitimate grace period explains it still being watched -
/// and confirm the view stops treating it as expected. Without this, the
/// view would be exactly as vacuous as the orphan check the ticket rejects:
/// green because the state it claims to catch cannot be produced.
#[tokio::test]
#[ignore]
async fn expected_watched_addresses_excludes_a_long_resolved_invoices_watch() {
    let service = create_test_service().await;

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

    let mut payment = test_payment(&invoice.id);
    payment.confirmed_at = Some(Utc::now() - Duration::days(2));
    PaymentWriter::upsert(&service, &payment).await.unwrap();

    let expected = service.get_expected_watched_addresses().await.unwrap();
    assert!(
        !expected.iter().any(|w| w.address == address),
        "a paid invoice's watch must not appear as expected once it is well \
         past any legitimate grace period, even though nothing has \
         deactivated the watched_addresses row yet"
    );
}

/// The view's `i.status IN (...)` clause names three "still live" statuses,
/// not one - a typo or an enum/DB string mismatch on either of the other two
/// would silently and permanently exclude every invoice in that status from
/// the expected set, which is exactly the "missed watch" failure the ticket
/// calls the worse fault. `expected_watched_addresses_includes_a_still_pending_invoices_watch`
/// only exercises `pending`; this covers the other two live branches.
#[tokio::test]
#[ignore]
async fn expected_watched_addresses_includes_a_processing_or_partially_paid_invoices_watch() {
    let service = create_test_service().await;

    for status in [InvoiceStatus::Processing, InvoiceStatus::PartiallyPaid] {
        let mut invoice = seeded_test_invoice(&service).await;
        invoice.expires_at = Utc::now() + Duration::hours(2);
        InvoiceWriter::upsert(&service, &invoice).await.unwrap();
        InvoiceWriter::update_status(&service, &invoice.id, status)
            .await
            .unwrap();

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
            expected.iter().any(|w| w.address == address),
            "an invoice in {status:?} must still appear as expected - the \
             view's status list names it live"
        );
    }
}

/// The mirror-image lag window: an invoice whose `expires_at` has already
/// passed, but `get_expired_for_cleanup` has not run yet, so `i.status` is
/// still `pending`. The invoice can still be paid at this instant and the
/// monitor is still, correctly, watching it. A view that additionally filters
/// on `wa.expires_at > NOW()` would drop this row and manufacture a false
/// "stale watch" report on every invoice that ever expires unpaid - the same
/// class of false positive the `is_active` scoping above exists to avoid,
/// reintroduced from the other side.
#[tokio::test]
#[ignore]
async fn expected_watched_addresses_includes_an_expired_but_not_yet_cleaned_up_invoices_watch() {
    let service = create_test_service().await;

    let mut invoice = seeded_test_invoice(&service).await;
    invoice.expires_at = Utc::now() - Duration::hours(1);
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
        expected.iter().any(|w| w.address == address),
        "an invoice past its expiry but still pending (cleanup has not run \
         yet) must still appear as expected - the monitor is still, \
         correctly, watching it"
    );
}

/// The same grace-window reasoning applies once `i.status` actually reaches
/// `expired`: `cleanup_expired_addresses` waits `unwatch_grace_period_secs`
/// after `expires_at` before unwatching, so an invoice that flipped to
/// `expired` moments ago must stay expected. Long after that window,
/// nothing legitimate explains a still-active watch, so it must drop out -
/// same shape as the paid-side pair of tests above, for the other status
/// this view treats specially.
#[tokio::test]
#[ignore]
async fn expected_watched_addresses_excludes_a_long_expired_invoices_watch() {
    let service = create_test_service().await;

    let mut invoice = seeded_test_invoice(&service).await;
    invoice.expires_at = Utc::now() - Duration::days(2);
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();
    InvoiceWriter::update_status(&service, &invoice.id, InvoiceStatus::Expired)
        .await
        .unwrap();

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
        !expected.iter().any(|w| w.address == address),
        "an invoice expired well past any legitimate grace period must not \
         appear as expected, even though nothing has deactivated the \
         watched_addresses row yet"
    );
}

/// `cancelled` gets no grace-window branch, unlike `paid`/`expired`:
/// `cleanup_cancelled_addresses` has no grace-period check at all, so the
/// only lag between `status` flipping and `is_active` catching up is the
/// cleanup job's own poll interval. A view that dropped `cancelled` from
/// "expected" the moment status flips (the earlier pass on this ticket did
/// exactly that, since `cancelled` was simply absent from the status list)
/// would report every routine cancellation as a false "stale watch" for
/// that window - the same class of false positive the `paid`/`expired`
/// branches above exist to avoid, missed for the third cleanup job the
/// ticket names in the same breath as the other two. Unlike those two,
/// there is no timestamp on `invoices` to bound a ceiling against and no
/// deliberate grace period to bound it to, so this asserts the watch stays
/// expected even long after cancellation, for as long as `is_active` says
/// the monitor is still, correctly, watching it.
#[tokio::test]
#[ignore]
async fn expected_watched_addresses_includes_a_cancelled_but_not_yet_cleaned_up_invoices_watch() {
    let service = create_test_service().await;

    let mut invoice = seeded_test_invoice(&service).await;
    invoice.expires_at = Utc::now() - Duration::days(2);
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();
    InvoiceWriter::update_status(&service, &invoice.id, InvoiceStatus::Cancelled)
        .await
        .unwrap();

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
        expected.iter().any(|w| w.address == address),
        "a cancelled invoice's watch must still appear as expected while \
         the cleanup job has not yet reached it - the monitor is still, \
         correctly, watching it"
    );
}

/// `late_paid` gets the same unconditional inclusion as `cancelled`, for a
/// stronger reason: no cleanup job ever selects it at all.
/// `InvoiceCleanupService::cleanup_addresses` only runs the expired/paid/
/// cancelled jobs, so a late-paid invoice's `watched_addresses` row never
/// has `is_active` flipped to `FALSE` and the monitor is never told to
/// unwatch it - Redis and `is_active` stay in agreement, correctly still
/// watching, forever. A view that excluded `late_paid` (as an earlier pass
/// on this ticket did, since it was simply absent from the status list)
/// would report that permanent, correct agreement as a stale watch with no
/// later cleanup pass to ever resolve the mismatch.
#[tokio::test]
#[ignore]
async fn expected_watched_addresses_includes_a_late_paid_invoices_watch() {
    let service = create_test_service().await;

    let mut invoice = seeded_test_invoice(&service).await;
    invoice.expires_at = Utc::now() - Duration::days(2);
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();
    InvoiceWriter::update_status(&service, &invoice.id, InvoiceStatus::LatePaid)
        .await
        .unwrap();

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

    let mut payment = test_payment(&invoice.id);
    payment.confirmed_at = Some(Utc::now() - Duration::days(2));
    PaymentWriter::upsert(&service, &payment).await.unwrap();

    let expected = service.get_expected_watched_addresses().await.unwrap();
    assert!(
        expected.iter().any(|w| w.address == address),
        "a late-paid invoice's watch must still appear as expected no \
         matter how long ago the late payment landed - no cleanup job will \
         ever deactivate it, so the monitor is correctly still watching it"
    );
}
