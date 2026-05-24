#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::super::*;
use ::types::{InvoiceQueryParams, InvoiceStatus};
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
