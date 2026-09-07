//! Invoice integration tests.

use chrono::{Duration, Utc};
use types::{InvoiceData, InvoiceQueryParams, InvoiceReader, InvoiceStatus, InvoiceWriter};

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

/// RCS-222: membership scoping has to be a real WHERE clause, not just a value
/// the handler computed and dropped.
///
/// The gate tests upstream prove the right *scope* comes back; this proves the
/// query honours it. That gap is where the interesting bugs live — the binds
/// here are positional, so a filter added in the wrong order silently applies
/// the wrong value to the wrong column and still returns rows.
#[tokio::test]
#[ignore]
async fn integration_invoice_query_scopes_to_a_set_of_stores() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Three stores, one invoice each, so "did it filter" and "did it filter to
    // the right ones" are different observations.
    let mine_a = seeded_test_invoice(&service).await;
    let mine_b = seeded_test_invoice(&service).await;
    let theirs = seeded_test_invoice(&service).await;
    for inv in [&mine_a, &mine_b, &theirs] {
        InvoiceWriter::upsert(&service, inv).await.unwrap();
    }

    let (total, rows) = InvoiceReader::query(
        &service,
        &InvoiceQueryParams::new().with_store_ids(vec![mine_a.store_id, mine_b.store_id]),
    )
    .await
    .unwrap();

    let ids: Vec<_> = rows.iter().map(|i| i.id.clone()).collect();
    assert!(ids.contains(&mine_a.id), "own store's invoice must appear");
    assert!(ids.contains(&mine_b.id), "own store's invoice must appear");
    assert!(
        !ids.contains(&theirs.id),
        "an invoice from a store outside the set must not appear"
    );
    assert_eq!(
        total, 2,
        "the count query must carry the same filter as the data query; \
         a mismatch here is what makes pagination lie"
    );

    // The empty case is the one that matters most: it must filter everything
    // out, not degrade into an unfiltered read of every store on the server.
    let (empty_total, empty_rows) =
        InvoiceReader::query(&service, &InvoiceQueryParams::new().with_store_ids(vec![]))
            .await
            .unwrap();
    assert_eq!(empty_total, 0, "no memberships must mean no rows");
    assert!(empty_rows.is_empty(), "no memberships must mean no rows");
}

/// RCS-231: search has to reach the SQL, and it has to reach *both* queries.
///
/// The pager takes `total` from the count query and the rows from the data
/// query. They are built separately, so a predicate added to one and not the
/// other does not fail - it reports a number that does not describe the page.
/// Every assertion below therefore checks the count and the rows together.
///
/// The store-scope half is the other trap: the search predicate is ANDed onto
/// the scope, never a replacement for it. The term here deliberately matches a
/// row in a store the caller cannot see (RCS-211, RCS-222).
#[tokio::test]
#[ignore]
async fn integration_invoice_search_is_scoped_and_counts_what_it_returns() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Unique per run: the test database is shared with every other test in the
    // file, so the term has to identify these rows and nothing else.
    let token = format!("rcs231{}", uuid::Uuid::new_v4().simple());

    let mut mine_matching = seeded_test_invoice(&service).await;
    mine_matching.metadata = Some(serde_json::json!({ "order_number": token }));
    let mine_other = InvoiceData {
        store_id: mine_matching.store_id,
        ..seeded_test_invoice(&service).await
    };
    // Same term, different tenant.
    let mut theirs_matching = seeded_test_invoice(&service).await;
    theirs_matching.metadata = Some(serde_json::json!({ "order_number": token }));

    for inv in [&mine_matching, &mine_other, &theirs_matching] {
        InvoiceWriter::upsert(&service, inv).await.unwrap();
    }

    // Unscoped: both tenants' matches, and a count that says so.
    let (total, rows) = InvoiceReader::query(
        &service,
        &InvoiceQueryParams::new().with_search(token.clone()),
    )
    .await
    .unwrap();
    assert_eq!(total, 2, "the search must reach the count query");
    assert_eq!(rows.len(), 2, "the search must reach the data query");

    // Scoped: the other tenant's match is gone, from the count as well.
    for scoped in [
        InvoiceQueryParams::new()
            .with_store_id(mine_matching.store_id)
            .with_search(token.clone()),
        InvoiceQueryParams::new()
            .with_store_ids(vec![mine_matching.store_id])
            .with_search(token.clone()),
    ] {
        let (total, rows) = InvoiceReader::query(&service, &scoped).await.unwrap();
        assert_eq!(total, 1, "search must not widen the store scope");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].id, mine_matching.id,
            "a match in a store the caller cannot see must stay invisible"
        );
    }

    // Blank is no filter, not an impossible one: the caller's other invoice
    // comes back too.
    let (total, rows) = InvoiceReader::query(
        &service,
        &InvoiceQueryParams::new()
            .with_store_ids(vec![mine_matching.store_id])
            .with_search("   "),
    )
    .await
    .unwrap();
    assert_eq!(total, 2, "a whitespace term must not filter anything");
    assert_eq!(rows.len(), 2);

    // A wildcard the user typed is a literal, not a pattern.
    let (total, _) = InvoiceReader::query(
        &service,
        &InvoiceQueryParams::new()
            .with_store_ids(vec![mine_matching.store_id])
            .with_search("%"),
    )
    .await
    .unwrap();
    assert_eq!(total, 0, "`%` must be escaped, not match every row");
}

/// RCS-231: the id predicate is anchored, and the currency one is not.
///
/// Pinned because the difference is a deliberate indexing decision (`%...%` can
/// never use an index; `term%` can), not an accident of how the SQL was typed.
#[tokio::test]
#[ignore]
async fn integration_invoice_search_anchors_the_id_but_not_the_currency() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let mut invoice = seeded_test_invoice(&service).await;
    invoice.currency = "USDC".to_string();
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    let id = invoice.id.0.clone();
    let scope = vec![invoice.store_id];

    let (total, rows) = InvoiceReader::query(
        &service,
        &InvoiceQueryParams::new()
            .with_store_ids(scope.clone())
            .with_search(id[..8].to_string()),
    )
    .await
    .unwrap();
    assert_eq!(total, 1, "an id prefix is what someone pasting an id types");
    assert_eq!(rows[0].id, invoice.id);

    let (total, _) = InvoiceReader::query(
        &service,
        &InvoiceQueryParams::new()
            .with_store_ids(scope.clone())
            .with_search(id[8..16].to_string()),
    )
    .await
    .unwrap();
    assert_eq!(
        total, 0,
        "the id predicate is anchored; a mid-string run must not match"
    );

    // Currency is a substring, and the match is case-insensitive.
    let (total, rows) = InvoiceReader::query(
        &service,
        &InvoiceQueryParams::new()
            .with_store_ids(scope)
            .with_search("SD"),
    )
    .await
    .unwrap();
    assert_eq!(total, 1, "currency is matched as a substring, case-folded");
    assert_eq!(rows[0].id, invoice.id);
}
