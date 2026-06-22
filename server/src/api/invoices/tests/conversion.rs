#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::super::*;
use rust_decimal::Decimal;

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
