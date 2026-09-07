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

/// RCS-231, the payments half: the search predicate reaches both the count and
/// the data query, and it is ANDed onto the store scope rather than replacing
/// it - the term below matches a payment in a store the caller cannot see.
///
/// The bind order is the thing this really guards. These binds are positional
/// and the search adds two of them in the middle of the list; getting the order
/// wrong applies the store filter to a `LIKE` pattern and still returns rows.
#[tokio::test]
#[ignore]
async fn integration_payment_search_is_scoped_and_counts_what_it_returns() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let mine = seeded_test_invoice(&service).await;
    let theirs = seeded_test_invoice(&service).await;
    for inv in [&mine, &theirs] {
        InvoiceWriter::upsert(&service, inv).await.unwrap();
    }

    // A hash prefix unique to this run, shared by both tenants' payments, plus
    // a sender fragment that only the caller's payment carries.
    let prefix = format!("0x{:016x}", uuid::Uuid::new_v4().as_u128() as u64);
    let mut mine_payment = test_payment(&mine.id);
    mine_payment.tx_hash = format!("{prefix}{:048x}", 1);
    mine_payment.from_address = Some(format!("0x{:040x}", uuid::Uuid::new_v4().as_u128()));
    let mut theirs_payment = test_payment(&theirs.id);
    theirs_payment.tx_hash = format!("{prefix}{:048x}", 2);
    for p in [&mine_payment, &theirs_payment] {
        PaymentWriter::upsert(&service, p).await.unwrap();
    }

    // Unscoped: both, counted.
    let (total, rows) = PaymentReader::query(
        &service,
        &PaymentQueryParams::new().with_search(prefix.clone()),
    )
    .await
    .unwrap();
    assert_eq!(total, 2, "the search must reach the count query");
    assert_eq!(rows.len(), 2, "the search must reach the data query");

    // Scoped, by either store filter: only the caller's, count included.
    for scoped in [
        PaymentQueryParams::new()
            .with_store_id(mine.store_id)
            .with_search(prefix.clone()),
        PaymentQueryParams::new()
            .with_store_ids(vec![mine.store_id])
            .with_search(prefix.clone()),
    ] {
        let (total, rows) = PaymentReader::query(&service, &scoped).await.unwrap();
        assert_eq!(total, 1, "search must not widen the store scope");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, mine_payment.id);
    }
}

/// RCS-231: the shape of each payment column's predicate, pinned.
///
/// Anchored on `tx_hash` (an identifier someone pastes whole, and the only
/// shape an index could ever serve), substring on `from_address`. Case-folded
/// either way, because a hash arrives in whatever case the explorer showed it.
#[tokio::test]
#[ignore]
async fn integration_payment_search_anchors_the_hash_but_not_the_sender() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let invoice = seeded_test_invoice(&service).await;
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    let prefix = format!("0x{:016x}", uuid::Uuid::new_v4().as_u128() as u64);
    let mut mine_payment = test_payment(&invoice.id);
    mine_payment.tx_hash = format!("{prefix}{:048x}", 1);
    mine_payment.from_address = Some(format!("0x{:040x}", uuid::Uuid::new_v4().as_u128()));
    PaymentWriter::upsert(&service, &mine_payment)
        .await
        .unwrap();

    let scope = vec![invoice.store_id];

    // A pasted hash arrives in whatever case the block explorer showed it.
    let (total, _) = PaymentReader::query(
        &service,
        &PaymentQueryParams::new()
            .with_store_ids(scope.clone())
            .with_search(prefix.to_uppercase()),
    )
    .await
    .unwrap();
    assert_eq!(total, 1, "hash matching is case-folded");

    // Anchored: a fragment from the middle of a hash is not a match.
    let (total, _) = PaymentReader::query(
        &service,
        &PaymentQueryParams::new()
            .with_store_ids(scope.clone())
            .with_search(prefix[6..].to_string()),
    )
    .await
    .unwrap();
    assert_eq!(
        total, 0,
        "the tx_hash predicate is anchored so it can use an index"
    );

    // Sender is a substring, and it is still scoped.
    let sender = mine_payment.from_address.clone().unwrap();
    let (total, rows) = PaymentReader::query(
        &service,
        &PaymentQueryParams::new()
            .with_store_ids(scope.clone())
            .with_search(sender[20..30].to_string()),
    )
    .await
    .unwrap();
    assert_eq!(total, 1, "from_address is matched as a substring");
    assert_eq!(rows[0].id, mine_payment.id);

    // The other two columns, same two shapes: the invoice id is anchored like
    // the hash, the asset symbol is a substring.
    for (term, shape) in [
        (invoice.id.0[..8].to_string(), "an invoice id prefix"),
        ("et".to_string(), "an asset symbol substring"),
    ] {
        let (total, _) = PaymentReader::query(
            &service,
            &PaymentQueryParams::new()
                .with_store_ids(scope.clone())
                .with_search(term),
        )
        .await
        .unwrap();
        assert_eq!(total, 1, "{shape} must match");
    }

    // Blank is no filter; a typed wildcard is a literal.
    let (blank_total, _) = PaymentReader::query(
        &service,
        &PaymentQueryParams::new()
            .with_store_ids(scope.clone())
            .with_search("  "),
    )
    .await
    .unwrap();
    assert_eq!(blank_total, 1, "a whitespace term must not filter anything");

    let (wildcard_total, _) = PaymentReader::query(
        &service,
        &PaymentQueryParams::new()
            .with_store_ids(scope)
            .with_search("%"),
    )
    .await
    .unwrap();
    assert_eq!(
        wildcard_total, 0,
        "`%` must be escaped, not match every row"
    );
}
