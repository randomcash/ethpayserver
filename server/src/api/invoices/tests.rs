#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use ::types::{InvoiceQueryParams, InvoiceStatus, StoreTokenPolicyEntry, TokenPolicyMode};
use rust_decimal::Decimal;
use uuid::Uuid;

#[test]
fn test_convert_to_crypto_basic() {
    // 100 USD at rate 0.0005 ETH/USD = 0.05 ETH = 50000000000000000 wei
    let rate = Decimal::from_str_exact("0.0005").unwrap();
    let result = convert_to_crypto_smallest_unit("100", rate, 18).unwrap();
    assert_eq!(result, "50000000000000000");
}

#[test]
fn test_convert_to_crypto_small_amount() {
    // 1 USD at rate 0.0005 ETH/USD = 0.0005 ETH = 500000000000000 wei
    let rate = Decimal::from_str_exact("0.0005").unwrap();
    let result = convert_to_crypto_smallest_unit("1", rate, 18).unwrap();
    assert_eq!(result, "500000000000000");
}

#[test]
fn test_convert_to_crypto_large_amount() {
    // 1,000,000 USD at rate 0.0005 ETH/USD = 500 ETH = 500000000000000000000 wei
    let rate = Decimal::from_str_exact("0.0005").unwrap();
    let result = convert_to_crypto_smallest_unit("1000000", rate, 18).unwrap();
    assert_eq!(result, "500000000000000000000");
}

#[test]
fn test_convert_to_crypto_fractional() {
    // 99.99 USD at rate 0.0005 ETH/USD = 0.049995 ETH
    let rate = Decimal::from_str_exact("0.0005").unwrap();
    let result = convert_to_crypto_smallest_unit("99.99", rate, 18).unwrap();
    // 0.049995 ETH = 49995000000000000 wei
    assert_eq!(result, "49995000000000000");
}

#[test]
fn test_convert_to_crypto_usdc_6_decimals() {
    // 100 USD at rate 1.0 USDC/USD = 100 USDC = 100000000 (6 decimals)
    let rate = Decimal::ONE;
    let result = convert_to_crypto_smallest_unit("100", rate, 6).unwrap();
    assert_eq!(result, "100000000");
}

#[test]
fn test_convert_rejects_zero_amount() {
    let rate = Decimal::from_str_exact("0.0005").unwrap();
    let result = convert_to_crypto_smallest_unit("0", rate, 18);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), "Amount must be positive");
}

#[test]
fn test_convert_rejects_negative_amount() {
    let rate = Decimal::from_str_exact("0.0005").unwrap();
    let result = convert_to_crypto_smallest_unit("-100", rate, 18);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), "Amount must be positive");
}

#[test]
fn test_convert_rejects_invalid_amount() {
    let rate = Decimal::from_str_exact("0.0005").unwrap();
    let result = convert_to_crypto_smallest_unit("abc", rate, 18);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), "Invalid amount");
}

#[test]
fn test_convert_floors_result() {
    // Ensure we floor (not round) to avoid overpaying
    // 1.001 USD at rate 0.0005 = 0.0005005 ETH = 500500000000000 wei
    let rate = Decimal::from_str_exact("0.0005").unwrap();
    let result = convert_to_crypto_smallest_unit("1.001", rate, 18).unwrap();
    assert_eq!(result, "500500000000000");
}

// Tests for same-asset conversion (no rate needed)
#[test]
fn test_convert_human_to_smallest_basic() {
    // 1.5 ETH = 1500000000000000000 wei
    let result = convert_human_to_smallest_unit("1.5", 18).unwrap();
    assert_eq!(result, "1500000000000000000");
}

#[test]
fn test_convert_human_to_smallest_usdc() {
    // 100 USDC = 100000000 (6 decimals)
    let result = convert_human_to_smallest_unit("100", 6).unwrap();
    assert_eq!(result, "100000000");
}

#[test]
fn test_convert_human_to_smallest_fractional() {
    // 0.001 ETH = 1000000000000000 wei
    let result = convert_human_to_smallest_unit("0.001", 18).unwrap();
    assert_eq!(result, "1000000000000000");
}

#[test]
fn test_convert_human_rejects_negative() {
    let result = convert_human_to_smallest_unit("-1", 18);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), "Amount must be positive");
}

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

// TX Hash Validation Tests
// =========================================================================

#[test]
fn test_valid_tx_hash() {
    assert!(is_valid_tx_hash(
        "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
    ));
}

#[test]
fn test_valid_tx_hash_uppercase() {
    assert!(is_valid_tx_hash(
        "0xABCDEF1234567890ABCDEF1234567890ABCDEF1234567890ABCDEF1234567890"
    ));
}

#[test]
fn test_valid_tx_hash_mixed_case() {
    assert!(is_valid_tx_hash(
        "0xaBcDeF1234567890AbCdEf1234567890aBcDeF1234567890AbCdEf1234567890"
    ));
}

#[test]
fn test_invalid_tx_hash_no_prefix() {
    assert!(!is_valid_tx_hash(
        "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
    ));
}

#[test]
fn test_invalid_tx_hash_too_short() {
    assert!(!is_valid_tx_hash("0xabcdef"));
}

#[test]
fn test_invalid_tx_hash_too_long() {
    assert!(!is_valid_tx_hash(
        "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef12345678901"
    ));
}

#[test]
fn test_invalid_tx_hash_non_hex() {
    assert!(!is_valid_tx_hash(
        "0xgggggg1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
    ));
}

#[test]
fn test_invalid_tx_hash_empty() {
    assert!(!is_valid_tx_hash(""));
}

// CSV writer tests

#[test]
fn test_csv_escape_no_special_chars() {
    assert_eq!(csv_escape_field("hello"), "hello");
    assert_eq!(csv_escape_field("12345"), "12345");
    assert_eq!(csv_escape_field(""), "");
}

#[test]
fn test_csv_escape_with_comma() {
    assert_eq!(csv_escape_field("hello,world"), "\"hello,world\"");
}

#[test]
fn test_csv_escape_with_quotes() {
    assert_eq!(csv_escape_field("say \"hi\""), "\"say \"\"hi\"\"\"");
}

#[test]
fn test_csv_escape_with_newline() {
    assert_eq!(csv_escape_field("line1\nline2"), "\"line1\nline2\"");
    assert_eq!(csv_escape_field("line1\rline2"), "\"line1\rline2\"");
}

#[test]
fn test_csv_escape_combined() {
    assert_eq!(csv_escape_field("a,b\"c\nd"), "\"a,b\"\"c\nd\"");
}

#[test]
fn test_csv_row_simple() {
    let row = csv_row(&["a", "b", "c"]);
    assert_eq!(row, "a,b,c\r\n");
}

#[test]
fn test_csv_row_with_escaping() {
    let row = csv_row(&["hello", "world,earth", "test"]);
    assert_eq!(row, "hello,\"world,earth\",test\r\n");
}

#[test]
fn test_csv_row_empty_fields() {
    let row = csv_row(&["id", "", "", "value"]);
    assert_eq!(row, "id,,,value\r\n");
}

#[test]
fn test_csv_row_unicode() {
    let row = csv_row(&["user@example.com", "Jos\u{00e9}", "100.00"]);
    assert_eq!(row, "user@example.com,Jos\u{00e9},100.00\r\n");
}

// =========================================================================
// Token policy filter tests (RCS-115)
// =========================================================================

fn make_pm(chain_id: u64, token_address: Option<&str>) -> data_service::StorePaymentMethod {
    data_service::StorePaymentMethod {
        id: uuid::Uuid::new_v4(),
        store_id: uuid::Uuid::new_v4(),
        chain_id,
        token_address: token_address.map(String::from),
        asset_symbol: "TEST".to_string(),
        decimals: 18,
        xpub: "xpub_test".to_string(),
        derivation_index: 0,
        enabled: true,
        created_at: chrono::Utc::now(),
    }
}

fn make_entry(chain_id: i64, token_address: Option<&str>) -> StoreTokenPolicyEntry {
    StoreTokenPolicyEntry {
        id: uuid::Uuid::new_v4(),
        policy_id: uuid::Uuid::new_v4(),
        chain_id,
        token_address: token_address.map(String::from),
        asset_symbol: "TEST".to_string(),
    }
}

fn make_policy(
    mode: TokenPolicyMode,
    entries: Vec<StoreTokenPolicyEntry>,
) -> data_service::StoreTokenPolicyWithEntries {
    data_service::StoreTokenPolicyWithEntries {
        id: uuid::Uuid::new_v4(),
        store_id: uuid::Uuid::new_v4(),
        mode,
        entries,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

#[test]
fn test_token_policy_allowlist_filters_correctly() {
    let mut methods = vec![
        make_pm(1, None),
        make_pm(1, Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")),
        make_pm(137, None),
    ];
    let policy = make_policy(TokenPolicyMode::Allowlist, vec![make_entry(1, None)]);
    apply_token_policy_filter(&mut methods, &policy);
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0].chain_id, 1);
    assert!(methods[0].token_address.is_none());
}

#[test]
fn test_token_policy_blocklist_filters_correctly() {
    let usdc = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
    let mut methods = vec![make_pm(1, None), make_pm(1, Some(usdc)), make_pm(137, None)];
    let policy = make_policy(TokenPolicyMode::Blocklist, vec![make_entry(1, Some(usdc))]);
    apply_token_policy_filter(&mut methods, &policy);
    assert_eq!(methods.len(), 2);
    assert!(methods.iter().all(|m| m.token_address.is_none()));
}

#[test]
fn test_token_policy_native_asset_handling() {
    let usdt = "0xdAC17F958D2ee523a2206206994597C13D831ec7";
    let mut methods = vec![make_pm(1, None), make_pm(1, Some(usdt))];
    let policy = make_policy(TokenPolicyMode::Allowlist, vec![make_entry(1, None)]);
    apply_token_policy_filter(&mut methods, &policy);
    assert_eq!(methods.len(), 1);
    assert!(methods[0].token_address.is_none());
}

#[test]
fn test_token_policy_allowlist_empty_entries_blocks_all() {
    let mut methods = vec![make_pm(1, None), make_pm(137, None)];
    let policy = make_policy(TokenPolicyMode::Allowlist, vec![]);
    apply_token_policy_filter(&mut methods, &policy);
    assert!(methods.is_empty());
}

#[test]
fn test_token_policy_blocklist_empty_entries_allows_all() {
    let mut methods = vec![make_pm(1, None), make_pm(137, None)];
    let policy = make_policy(TokenPolicyMode::Blocklist, vec![]);
    apply_token_policy_filter(&mut methods, &policy);
    assert_eq!(methods.len(), 2);
}
