#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::super::*;
use ::types::{InvoiceQueryParams, InvoiceStatus, StoreId};
use uuid::Uuid;

// =========================================================================
// ListInvoicesQuery deserialization
// =========================================================================

#[test]
fn test_query_deserializes_all_fields() {
    let q: ListInvoicesQuery = serde_json::from_value(serde_json::json!({
        "store_id": "00000000-0000-0000-0000-000000000001",
        "status": "paid",
        "currency": "USD",
        "limit": 10,
        "offset": 5
    }))
    .unwrap();
    assert_eq!(
        q.store_id.unwrap(),
        "00000000-0000-0000-0000-000000000001"
            .parse::<Uuid>()
            .unwrap()
    );
    assert_eq!(q.status.unwrap(), "paid");
    assert_eq!(q.currency.unwrap(), "USD");
    assert_eq!(q.limit.unwrap(), 10);
    assert_eq!(q.offset.unwrap(), 5);
}

#[test]
fn test_query_deserializes_empty() {
    let q: ListInvoicesQuery = serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(q.store_id.is_none());
    assert!(q.status.is_none());
    assert!(q.currency.is_none());
    assert!(q.limit.is_none());
    assert!(q.offset.is_none());
}

#[test]
fn test_query_deserializes_currency_only() {
    let q: ListInvoicesQuery =
        serde_json::from_value(serde_json::json!({"currency": "ETH"})).unwrap();
    assert!(q.store_id.is_none());
    assert!(q.status.is_none());
    assert_eq!(q.currency.unwrap(), "ETH");
}

#[test]
fn test_query_deserializes_status_only() {
    let q: ListInvoicesQuery =
        serde_json::from_value(serde_json::json!({"status": "pending"})).unwrap();
    assert_eq!(q.status.unwrap(), "pending");
    assert!(q.currency.is_none());
}

#[test]
fn test_query_deserializes_both_filters() {
    let q: ListInvoicesQuery =
        serde_json::from_value(serde_json::json!({"status": "expired", "currency": "USDC"}))
            .unwrap();
    assert_eq!(q.status.unwrap(), "expired");
    assert_eq!(q.currency.unwrap(), "USDC");
}

// =========================================================================
// Status string parsing (handler uses .parse::<InvoiceStatus>())
// =========================================================================

#[test]
fn test_status_parse_all_variants() {
    let cases = [
        ("pending", InvoiceStatus::Pending),
        ("processing", InvoiceStatus::Processing),
        ("partially_paid", InvoiceStatus::PartiallyPaid),
        ("paid", InvoiceStatus::Paid),
        ("expired", InvoiceStatus::Expired),
        ("cancelled", InvoiceStatus::Cancelled),
        ("canceled", InvoiceStatus::Cancelled),
        ("refunded", InvoiceStatus::Refunded),
        ("late_paid", InvoiceStatus::LatePaid),
    ];
    for (input, expected) in cases {
        let parsed: InvoiceStatus = input.parse().unwrap();
        assert_eq!(parsed, expected, "failed for input: {input}");
    }
}

#[test]
fn test_status_parse_invalid() {
    let result = "bogus".parse::<InvoiceStatus>();
    assert!(result.is_err());
}

// =========================================================================
// InvoiceQueryParams builder wiring
// =========================================================================

#[test]
fn test_query_params_defaults() {
    let params = InvoiceQueryParams::new();
    assert!(params.store_id.is_none());
    assert!(params.status.is_none());
    assert!(params.currency.is_none());
    assert_eq!(params.limit, 50);
    assert_eq!(params.offset, 0);
}

#[test]
fn test_query_params_with_currency() {
    let params = InvoiceQueryParams::new().with_currency("EUR");
    assert_eq!(params.currency.as_deref(), Some("EUR"));
}

#[test]
fn test_query_params_with_status_and_currency() {
    let params = InvoiceQueryParams::new()
        .with_status(InvoiceStatus::Paid)
        .with_currency("USD");
    assert_eq!(params.status, Some(InvoiceStatus::Paid));
    assert_eq!(params.currency.as_deref(), Some("USD"));
}

#[test]
fn test_query_params_pagination_override() {
    let params = InvoiceQueryParams::new().with_limit(25).with_offset(10);
    assert_eq!(params.limit, 25);
    assert_eq!(params.offset, 10);
}

// =========================================================================
// InvoiceListResponse / InvoiceResponse serialization
// =========================================================================

#[test]
fn test_invoice_list_response_serializes() {
    let resp = InvoiceListResponse {
        total: 0,
        invoices: vec![],
    };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["total"], 0);
    assert!(json["invoices"].as_array().unwrap().is_empty());
}

// =========================================================================
// List search
// =========================================================================

#[test]
fn test_query_deserializes_search() {
    let q: ListInvoicesQuery =
        serde_json::from_value(serde_json::json!({"search": "0xdead"})).unwrap();
    assert_eq!(q.search.unwrap(), "0xdead");

    let q: ListPaymentsQuery =
        serde_json::from_value(serde_json::json!({"search": "USDC"})).unwrap();
    assert_eq!(q.search.unwrap(), "USDC");

    // Omitting it is not the same as sending an empty one, and neither filters.
    let q: ListInvoicesQuery = serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(q.search.is_none());
}

/// The filter builders are shared with the CSV export on purpose: an export
/// that ignores a filter the list applied downloads something other than what
/// is on screen. Both sides of that are asserted here because there is only one
/// builder to assert.
#[test]
fn search_reaches_the_shared_filter_builders() {
    let scope = StoreScope::All;

    let params = build_invoice_filter_params(&scope, None, None, Some("0xDEAD")).unwrap();
    assert_eq!(params.search_term(), Some("0xDEAD"));

    let params = build_payment_filter_params(&scope, None, Some("usdc")).unwrap();
    assert_eq!(params.search_term(), Some("usdc"));
}

/// A cleared search box must widen the list back out, not empty it.
#[test]
fn blank_search_reaches_the_builder_as_no_filter() {
    let scope = StoreScope::All;

    for blank in ["", "   "] {
        let params = build_invoice_filter_params(&scope, None, None, Some(blank)).unwrap();
        assert_eq!(params.search_term(), None, "invoice search {blank:?}");

        let params = build_payment_filter_params(&scope, None, Some(blank)).unwrap();
        assert_eq!(params.search_term(), None, "payment search {blank:?}");
    }
}

/// Search is ANDed onto the store scope, never a replacement for it. The row
/// level of this is proved against a real database in `data-service`; what is
/// checked here is that the scope survives the builder at all - dropping it
/// here is the cross-store leak with extra steps.
#[test]
fn search_does_not_displace_the_store_scope() {
    let mine = StoreId(Uuid::new_v4());

    let params =
        build_invoice_filter_params(&StoreScope::One(mine), None, None, Some("usdc")).unwrap();
    assert_eq!(params.store_id, Some(mine));
    assert_eq!(params.search_term(), Some("usdc"));

    let params =
        build_payment_filter_params(&StoreScope::Membership(vec![mine]), None, Some("0xdead"))
            .unwrap();
    assert_eq!(params.store_ids, Some(vec![mine]));
    assert_eq!(params.search_term(), Some("0xdead"));

    // The empty membership stays a filter that matches nothing, search or not.
    let params =
        build_payment_filter_params(&StoreScope::Membership(vec![]), None, Some("0xdead")).unwrap();
    assert_eq!(params.store_ids, Some(vec![]));
}

// =========================================================================
// Invalid filter values carry a reason
// =========================================================================

/// A bare `StatusCode::BAD_REQUEST` reaches the client as `ApiError::Http {
/// status: 400, message: "" }` - indistinguishable from any other 400 these
/// endpoints can return. Both list pages used to swallow every 400 as "pick a
/// store" whenever no store was selected, which would have mis-rendered an
/// invalid filter as that empty state instead of showing the real error. The
/// server has to hand the client something to key on instead of the bare code.
#[tokio::test]
async fn invalid_status_filter_carries_a_reason_the_client_can_key_on() {
    async fn body_of(err: impl axum::response::IntoResponse) -> (StatusCode, String) {
        let response = err.into_response();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    let scope = StoreScope::All;

    let err = build_invoice_filter_params(&scope, Some("bogus"), None, None).unwrap_err();
    let (status, body) = body_of(err).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        !body.is_empty(),
        "invoice status 400 must carry a reason, not an empty body"
    );

    let err = build_payment_filter_params(&scope, Some("bogus"), None).unwrap_err();
    let (status, body) = body_of(err).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        !body.is_empty(),
        "payment status 400 must carry a reason, not an empty body"
    );
}

// =========================================================================
// Handler wiring: GET /payments/{id} store backfill
// =========================================================================

/// `From<PaymentData> for PaymentResponse` (in payserver-commons) always
/// leaves `store_id`/`store_name` `None` - only the list handler backfilled
/// them, so `GET /payments/{id}` returned null for both no matter what the
/// OpenAPI schema promised. Handlers cannot be instantiated in a unit test
/// (`PgAppState` is pinned to the concrete Postgres service - see the module
/// doc on `store_scope.rs`), so this checks the wiring the cheap way: the
/// single-payment handler must not return the bare, un-backfilled conversion.
#[test]
fn get_payment_backfills_store_fields_from_the_invoice_it_already_looked_up() {
    let src = include_str!("../payments.rs");
    assert!(
        !src.contains("Ok(Json(payment.into()))"),
        "get_payment must not return the bare PaymentData conversion - it \
         already looks up the invoice for the membership check, so store_id \
         and store_name should come along for free"
    );
    assert!(
        src.contains("response.store_id = Some(invoice.store_id"),
        "get_payment must set store_id from the invoice it already holds"
    );
}

/// The list and the export must feed the same field into that shared builder.
/// Handlers cannot be instantiated in a unit test (`PgAppState` is pinned to
/// the concrete Postgres service), so this checks the wiring the cheap way, in
/// the same spirit as the nil-sentinel scan in `store_scope.rs`.
#[test]
fn list_and_export_handlers_both_pass_search_to_the_builder() {
    //
    // Counted, not just looked for: `csv_export.rs` holds both exports, so a
    // bare `contains` still passes with one of them silently dropped.
    for (name, src, call_sites) in [
        ("list.rs", include_str!("../list.rs"), 1),
        ("payments.rs", include_str!("../payments.rs"), 1),
        ("csv_export.rs", include_str!("../csv_export.rs"), 2),
    ] {
        assert_eq!(
            src.matches("query.search.as_deref()").count(),
            call_sites,
            "{name} must pass the search term into the shared filter builder at \
             every list and export call site, or the export downloads something \
             other than what is on screen"
        );
    }
}
