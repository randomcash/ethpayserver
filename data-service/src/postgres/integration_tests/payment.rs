//! Payment integration tests.

use chrono::Utc;
use types::{InvoiceReader, InvoiceWriter, PaymentQueryParams, PaymentReader, PaymentWriter};

use crate::{PaymentTxIndexWriter, WebhookOutboxReader, WebhookOutboxWriter};

use super::{assert_amount_eq, create_test_service, seeded_test_invoice, test_payment};

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

/// Membership scoping, the payments half. Worth its own test rather than trusting the
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

/// Search, the payments half: the search predicate reaches both the count and
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

/// The shape of each payment column's predicate, pinned.
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

/// A batching contract, a multicall, or an exchange sweep can pay two
/// different watched addresses in a single transaction. `unique_payment_tx`
/// used to be `(tx_hash, chain_id)` alone, so the second transfer's
/// `ON CONFLICT` silently overwrote the first instead of inserting - one of
/// the two payments ceased to exist. `tx_index` (the EVM log index) is the
/// third column that tells the two transfers apart.
///
/// Against the pre-fix schema (`unique_payment_tx (tx_hash, chain_id)`, no
/// `tx_index` column) this produces one row and the second amount is lost.
#[tokio::test]
#[ignore]
async fn integration_payment_upsert_keeps_two_transfers_in_one_tx() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let invoice = seeded_test_invoice(&service).await;
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // Two transfers in the same transaction: same tx_hash and chain_id, a
    // different log index each, paying different amounts to different
    // addresses.
    let shared_tx_hash = format!("0x{:064x}", uuid::Uuid::new_v4().as_u128());
    let mut first = test_payment(&invoice.id);
    first.tx_hash = shared_tx_hash.clone();
    first.amount = "1000000000000000000".to_string();
    let mut second = test_payment(&invoice.id);
    second.tx_hash = shared_tx_hash;
    second.amount = "2000000000000000000".to_string();

    PaymentTxIndexWriter::upsert_with_tx_index(&service, &first, 0)
        .await
        .unwrap();
    PaymentTxIndexWriter::upsert_with_tx_index(&service, &second, 1)
        .await
        .unwrap();

    let payments = PaymentReader::get_for_invoice(&service, &invoice.id)
        .await
        .unwrap();
    assert_eq!(
        payments.len(),
        2,
        "two transfers batched into one tx must produce two rows, not one \
         overwritten by the other"
    );

    let fetched_first = payments
        .iter()
        .find(|p| p.id == first.id)
        .expect("the first transfer must survive the second's upsert");
    let fetched_second = payments
        .iter()
        .find(|p| p.id == second.id)
        .expect("the second transfer must have been inserted, not merged into the first");
    assert_amount_eq(
        &fetched_first.amount,
        &first.amount,
        "first transfer's amount",
    );
    assert_amount_eq(
        &fetched_second.amount,
        &second.amount,
        "second transfer's amount",
    );
}

/// The payment row and its webhook notification obligation are written in
/// one transaction. Verified here as "both exist together against a real
/// Postgres", which a unit test against the in-memory double cannot prove.
#[tokio::test]
#[ignore]
async fn integration_upsert_with_tx_index_and_obligation_writes_both_rows() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let invoice = seeded_test_invoice(&service).await;
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    let payment = test_payment(&invoice.id);
    PaymentTxIndexWriter::upsert_with_tx_index_and_obligation(
        &service,
        &payment,
        0,
        "payment_detected",
    )
    .await
    .unwrap();

    let fetched = PaymentReader::get(&service, payment.id)
        .await
        .unwrap()
        .expect("payment row must exist");
    assert_eq!(fetched.id, payment.id);

    let obligations = WebhookOutboxReader::claim_undispatched_obligations(&service, 10, 30)
        .await
        .unwrap();
    let obligation = obligations
        .iter()
        .find(|o| o.payment_id == payment.id)
        .expect("obligation for this payment must be recorded");
    assert_eq!(obligation.invoice_id, invoice.id.as_str());
    assert_eq!(obligation.event_type, "payment_detected");

    WebhookOutboxWriter::mark_obligation_dispatched(&service, obligation.id)
        .await
        .unwrap();
    let remaining = WebhookOutboxReader::claim_undispatched_obligations(&service, 10, 30)
        .await
        .unwrap();
    assert!(
        !remaining.iter().any(|o| o.id == obligation.id),
        "a dispatched obligation must not be read again"
    );
}

/// A redelivered `PaymentDetected` (delivery is documented as at-least-once)
/// re-runs this same call with a fresh `payment.id` but the same
/// `(chain_id, tx_hash, tx_index)`. The upsert updates the *existing* payment
/// row rather than inserting a new one, so the obligation's `payment_id`
/// foreign key must point at that existing row's real id - not the fresh,
/// never-persisted one the second call generated - or the insert would
/// violate the foreign key. The unique constraint on `(payment_id,
/// event_type)` must also stop the redelivery from queuing a second
/// obligation for a payment already on file.
#[tokio::test]
#[ignore]
async fn integration_redelivered_payment_reuses_the_original_row_id_and_does_not_duplicate_the_obligation()
 {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let invoice = seeded_test_invoice(&service).await;
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    let mut first = test_payment(&invoice.id);
    first.credited_amount = Some("100.5".to_string());
    PaymentTxIndexWriter::upsert_with_tx_index_and_obligation(
        &service,
        &first,
        0,
        "payment_detected",
    )
    .await
    .unwrap();

    // Same transfer, redelivered: same tx_hash/chain_id/tx_index, but a fresh
    // random id - exactly what a second delivery of the same monitor event
    // produces.
    let mut redelivered = test_payment(&invoice.id);
    redelivered.tx_hash = first.tx_hash.clone();
    redelivered.chain_id = first.chain_id.clone();
    redelivered.credited_amount = first.credited_amount.clone();
    assert_ne!(
        redelivered.id, first.id,
        "the redelivery must generate its own fresh id, as a real redelivery does"
    );

    PaymentTxIndexWriter::upsert_with_tx_index_and_obligation(
        &service,
        &redelivered,
        0,
        "payment_detected",
    )
    .await
    .expect("a redelivery must not violate the obligation's foreign key");

    let payments = PaymentReader::get_for_invoice(&service, &invoice.id)
        .await
        .unwrap();
    assert_eq!(
        payments.len(),
        1,
        "a redelivery of the same transfer must update the existing row, not add one"
    );
    let real_id = payments[0].id;

    // The batch must be larger than any backlog other tests leave on the shared
    // database, or this invoice's obligation is simply not in it.
    let obligations = WebhookOutboxReader::claim_undispatched_obligations(&service, 10_000, 30)
        .await
        .unwrap();
    let matching: Vec<_> = obligations
        .iter()
        .filter(|o| o.invoice_id == invoice.id.as_str())
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "a redelivery must not queue a second obligation for the same payment"
    );
    assert_eq!(
        matching[0].payment_id, real_id,
        "the obligation must name the real, persisted payment row"
    );

    // The invoice total is the sum of payment rows, so a redelivery that
    // updated the existing row must leave it at one payment's credit, not two.
    let fetched = InvoiceReader::get(&service, &invoice.id)
        .await
        .unwrap()
        .unwrap();
    assert_amount_eq(
        &fetched.amount_received,
        "100.5",
        "amount_received after a redelivery",
    );
}

/// The atomicity claim is the entire point of the transactional outbox: if
/// the obligation insert fails, the payment insert in the same transaction
/// must not survive. A trigger that unconditionally fails every insert into
/// `webhook_outbox` stands in for that failure, so this exercises the real
/// production function end to end rather than a hand-rolled mimic of it - a
/// regression that split the two writes across separate connections or
/// separate `execute()` calls against the pool (instead of sharing one
/// `&mut *tx`) would still commit the payment row here and fail this test.
#[tokio::test]
#[ignore]
async fn integration_obligation_insert_failure_rolls_back_the_payment_row() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let invoice = seeded_test_invoice(&service).await;
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    sqlx::query(
        r#"
        CREATE OR REPLACE FUNCTION integration_test_fail_webhook_outbox_insert()
        RETURNS trigger AS $$
        BEGIN
            RAISE EXCEPTION 'fault injected by atomicity integration test';
        END;
        $$ LANGUAGE plpgsql
        "#,
    )
    .execute(&service.pool)
    .await
    .unwrap();
    sqlx::query(
        r#"
        CREATE TRIGGER integration_test_fail_webhook_outbox_insert_trigger
        BEFORE INSERT ON webhook_outbox
        FOR EACH ROW EXECUTE FUNCTION integration_test_fail_webhook_outbox_insert()
        "#,
    )
    .execute(&service.pool)
    .await
    .unwrap();

    let payment = test_payment(&invoice.id);
    let result = PaymentTxIndexWriter::upsert_with_tx_index_and_obligation(
        &service,
        &payment,
        0,
        "payment_detected",
    )
    .await;

    // Undo the fault injection before asserting, so a failed assertion below
    // does not leave every other test in this suite unable to insert into
    // webhook_outbox.
    sqlx::query(
        "DROP TRIGGER integration_test_fail_webhook_outbox_insert_trigger ON webhook_outbox",
    )
    .execute(&service.pool)
    .await
    .unwrap();
    sqlx::query("DROP FUNCTION integration_test_fail_webhook_outbox_insert()")
        .execute(&service.pool)
        .await
        .unwrap();

    assert!(
        result.is_err(),
        "the injected fault must surface as an error, not be swallowed"
    );

    let fetched = PaymentReader::get(&service, payment.id).await.unwrap();
    assert!(
        fetched.is_none(),
        "a failed obligation insert must roll back the payment row written in the same \
         transaction, not leave a committed payment with no obligation"
    );
}

/// The property the drain's `FOR UPDATE SKIP LOCKED` claim exists for: this
/// server runs more than one instance for availability, and a plain read of
/// `webhook_outbox` would let two of them claim and act on the same
/// obligation, queuing the same webhook twice. A claimed-but-not-yet-expired
/// obligation must be invisible to another claim call, and an expired one
/// must become claimable again so a claimant that dies mid-dispatch does not
/// lose the obligation permanently.
#[tokio::test]
#[ignore]
async fn integration_claim_undispatched_obligations_hides_claimed_rows_until_expiry() {
    let service = create_test_service().await.expect("DATABASE_URL required");
    let invoice = seeded_test_invoice(&service).await;
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // First obligation: claimed with a long visibility window, standing in
    // for one drain instance that has picked it up and is still working on
    // it.
    let held = test_payment(&invoice.id);
    PaymentTxIndexWriter::upsert_with_tx_index_and_obligation(
        &service,
        &held,
        0,
        "payment_detected",
    )
    .await
    .unwrap();
    let first_claim = WebhookOutboxReader::claim_undispatched_obligations(&service, 10, 60)
        .await
        .unwrap();
    assert_eq!(
        first_claim
            .iter()
            .filter(|o| o.payment_id == held.id)
            .count(),
        1,
        "the obligation must be claimable the first time"
    );

    let second_claim = WebhookOutboxReader::claim_undispatched_obligations(&service, 10, 60)
        .await
        .unwrap();
    assert!(
        !second_claim.iter().any(|o| o.payment_id == held.id),
        "a second claimant must not see an obligation still inside its visibility window - \
         seeing it would mean two drain instances can queue the same webhook twice"
    );

    // Second obligation, claimed with zero visibility: the deadline is
    // already in the past by the time the next call runs, simulating a
    // claimant that died before it could mark the obligation dispatched.
    let abandoned = test_payment(&invoice.id);
    PaymentTxIndexWriter::upsert_with_tx_index_and_obligation(
        &service,
        &abandoned,
        0,
        "payment_detected",
    )
    .await
    .unwrap();
    let claimed_briefly = WebhookOutboxReader::claim_undispatched_obligations(&service, 10, 0)
        .await
        .unwrap();
    assert!(
        claimed_briefly.iter().any(|o| o.payment_id == abandoned.id),
        "the obligation must be claimable before its (immediately-expiring) deadline passes"
    );

    let reclaimed = WebhookOutboxReader::claim_undispatched_obligations(&service, 10, 60)
        .await
        .unwrap();
    assert!(
        reclaimed.iter().any(|o| o.payment_id == abandoned.id),
        "an obligation whose claim has expired must become claimable again, or an abandoned \
         obligation is lost rather than retried"
    );
}
