#![allow(clippy::unwrap_used, clippy::expect_used)]
mod helpers;
mod payment_confirmed;
mod payment_detected;
mod reorg;

use std::sync::Arc;

use data_service::InMemoryDataService;
use evm::monitor::bridge::MemoryBridge;

use super::*;
use helpers::{create_test_consumer, native_symbol};

#[test]
fn test_native_symbol() {
    use types::ChainId;
    assert_eq!(native_symbol(&ChainId::evm(1)), "ETH");
    assert_eq!(native_symbol(&ChainId::evm(42161)), "ETH");
    assert_eq!(native_symbol(&ChainId::evm(10)), "ETH");
    assert_eq!(native_symbol(&ChainId::evm(8453)), "ETH");
    assert_eq!(native_symbol(&ChainId::evm(137)), "POL");
    assert_eq!(native_symbol(&ChainId::evm(43114)), "AVAX");
    assert_eq!(native_symbol(&ChainId::evm(56)), "BNB");
    assert_eq!(native_symbol(&ChainId::evm(250)), "FTM");
    assert_eq!(native_symbol(&ChainId::evm(100)), "xDAI");

    // A chain this table does not know, including a non-EVM one. It renders
    // rather than failing, which is the property the closed enum lacked.
    assert_eq!(native_symbol(&ChainId::evm(999_999)), "UNKNOWN");
    assert_eq!(
        native_symbol(&ChainId::parse("tron:728126428").unwrap()),
        "UNKNOWN"
    );
}

#[test]
fn test_event_consumer_error_display() {
    let db_err = EventConsumerError::Database(types::RepositoryError::NotFound("test".into()));
    assert!(db_err.to_string().contains("database error"));

    let data_err = EventConsumerError::InvalidData("bad data".into());
    assert!(data_err.to_string().contains("invalid data"));
}

/// 2^96 base units - one past the largest integer `rust_decimal::Decimal`
/// (96-bit mantissa) can represent exactly. `convert_smallest_to_human` is the
/// function RCS-286 names as the bug site; a version that round-tripped
/// through `Decimal` here would fail to parse this amount at all rather than
/// merely round it (see the sibling handler tests), so this asserts the exact
/// digit string comes back unchanged.
#[test]
fn test_convert_smallest_to_human_exact_beyond_decimal_precision() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds, bridge);

    let raw = "79228162514264337593543950336"; // 2^96
    let human = consumer.convert_smallest_to_human(raw, 0).unwrap();
    assert_eq!(human, raw);
}

/// Same amount and function as above, but through the decimals=18 path a
/// real EVM payment actually takes (`raw_amount / 10^18`), with `rate = "1"`
/// so the invoice-currency division is a no-op and any precision loss can
/// only come from the parse or the power-of-ten divide this ticket is about.
#[test]
fn test_convert_payment_to_invoice_currency_exact_beyond_decimal_precision() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds, bridge);

    // 2^96 followed by 18 zeros, i.e. 2^96 expressed as wei of an
    // 18-decimal token.
    let raw = "79228162514264337593543950336000000000000000000";
    let converted = consumer
        .convert_payment_to_invoice_currency(raw, "1", 18)
        .unwrap();
    assert_eq!(converted, "79228162514264337593543950336");
}
