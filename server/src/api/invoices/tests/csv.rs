#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::super::*;

// =========================================================================
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

// =========================================================================
// CSV writer tests
// =========================================================================

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
