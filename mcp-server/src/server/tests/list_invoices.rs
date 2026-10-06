//! `do_list_invoices` — the tool an agent calls to review a store's invoices.

use types::InvoiceStatus;

use crate::server::ListInvoicesArgs;
use crate::testkit::{StubRateProvider, TestHarness, parse_ok};

fn args(store_id: types::StoreId) -> ListInvoicesArgs {
    ListInvoicesArgs {
        store_id: store_id.0.to_string(),
        status: None,
        currency: None,
        limit: None,
        offset: None,
    }
}

#[tokio::test]
async fn lists_the_store_invoices_with_a_total() {
    let h = TestHarness::new(StubRateProvider::usd_eth());
    h.seed_invoice(h.store_id, "USD", "10.00", InvoiceStatus::Pending)
        .await;
    h.seed_invoice(h.store_id, "EUR", "20.00", InvoiceStatus::Paid)
        .await;

    let json = parse_ok(h.server.do_list_invoices(args(h.store_id)).await);

    assert_eq!(json["total"], 2);
    let invoices = json["invoices"].as_array().unwrap();
    assert_eq!(invoices.len(), 2);
    for invoice in invoices {
        for field in [
            "id",
            "currency",
            "status",
            "amount",
            "amount_received",
            "created_at",
            "expires_at",
        ] {
            assert!(!invoice[field].is_null(), "missing {field} in {invoice}");
        }
    }
}

#[tokio::test]
async fn does_not_leak_invoices_from_another_store() {
    let h = TestHarness::new(StubRateProvider::usd_eth());
    h.seed_invoice(h.store_id, "USD", "10.00", InvoiceStatus::Pending)
        .await;
    h.seed_invoice(
        TestHarness::foreign_store_id(),
        "USD",
        "999.00",
        InvoiceStatus::Pending,
    )
    .await;

    let json = parse_ok(h.server.do_list_invoices(args(h.store_id)).await);

    assert_eq!(json["total"], 1);
    assert_eq!(json["invoices"].as_array().unwrap()[0]["amount"], "10.00");
}

#[tokio::test]
async fn rejects_a_store_outside_the_session_scope() {
    let h = TestHarness::new(StubRateProvider::usd_eth());

    let err = h
        .server
        .do_list_invoices(args(TestHarness::foreign_store_id()))
        .await
        .unwrap_err();

    assert!(err.starts_with("Unauthorized"), "got: {err}");
}

#[tokio::test]
async fn rejects_a_malformed_store_id() {
    let h = TestHarness::new(StubRateProvider::usd_eth());
    let mut args = args(h.store_id);
    args.store_id = "nope".to_string();

    assert_eq!(
        h.server.do_list_invoices(args).await.unwrap_err(),
        "Invalid store_id UUID"
    );
}

#[tokio::test]
async fn filters_by_status() {
    let h = TestHarness::new(StubRateProvider::usd_eth());
    h.seed_invoice(h.store_id, "USD", "10.00", InvoiceStatus::Pending)
        .await;
    h.seed_invoice(h.store_id, "USD", "20.00", InvoiceStatus::Paid)
        .await;

    let mut args = args(h.store_id);
    args.status = Some("paid".to_string());
    let json = parse_ok(h.server.do_list_invoices(args).await);

    assert_eq!(json["total"], 1);
    assert_eq!(json["invoices"].as_array().unwrap()[0]["status"], "paid");
}

#[tokio::test]
async fn filters_by_currency() {
    let h = TestHarness::new(StubRateProvider::usd_eth());
    h.seed_invoice(h.store_id, "USD", "10.00", InvoiceStatus::Pending)
        .await;
    h.seed_invoice(h.store_id, "EUR", "20.00", InvoiceStatus::Pending)
        .await;

    let mut args = args(h.store_id);
    args.currency = Some("EUR".to_string());
    let json = parse_ok(h.server.do_list_invoices(args).await);

    assert_eq!(json["total"], 1);
    assert_eq!(json["invoices"].as_array().unwrap()[0]["currency"], "EUR");
}

#[tokio::test]
async fn rejects_an_unknown_status_filter() {
    let h = TestHarness::new(StubRateProvider::usd_eth());
    let mut args = args(h.store_id);
    args.status = Some("almost_paid".to_string());

    assert_eq!(
        h.server.do_list_invoices(args).await.unwrap_err(),
        "Invalid status filter"
    );
}

#[tokio::test]
async fn paginates_while_reporting_the_full_total() {
    let h = TestHarness::new(StubRateProvider::usd_eth());
    for amount in ["1.00", "2.00", "3.00"] {
        h.seed_invoice(h.store_id, "USD", amount, InvoiceStatus::Pending)
            .await;
    }

    let mut first_page = args(h.store_id);
    first_page.limit = Some(2);
    let json = parse_ok(h.server.do_list_invoices(first_page).await);
    assert_eq!(json["total"], 3, "total counts all matches, not the page");
    assert_eq!(json["invoices"].as_array().unwrap().len(), 2);

    let mut second_page = args(h.store_id);
    second_page.limit = Some(2);
    second_page.offset = Some(2);
    let json = parse_ok(h.server.do_list_invoices(second_page).await);
    assert_eq!(json["total"], 3);
    assert_eq!(json["invoices"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn returns_an_empty_list_for_a_store_with_no_invoices() {
    let h = TestHarness::new(StubRateProvider::usd_eth());

    let json = parse_ok(h.server.do_list_invoices(args(h.store_id)).await);

    assert_eq!(json["total"], 0);
    assert!(json["invoices"].as_array().unwrap().is_empty());
}
