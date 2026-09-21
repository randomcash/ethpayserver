#![allow(clippy::unwrap_used, clippy::expect_used)]
mod apply_halts_on_failure;
mod durable_resume;
mod helpers;
mod lineage_break;
mod multi_chain_resume;
mod out_of_range_retry;
mod own_store_payments;
mod payment_confirmed;
mod payment_detected;
mod reconcile_cursors;
mod reorg;
mod resume_failure_hook;
mod resume_uses_persisted_cursor;

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
/// function where the overflow was found; a version that round-tripped
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

/// One wei of an 18-decimal token must not reach the WebSocket as `1E-18`.
///
/// `BigDecimal`'s `Display` switches to scientific notation past five leading
/// zeros; `rust_decimal`'s never did. The value produced here is broadcast to
/// the checkout and dashboard as a string, so the notation is user-visible.
/// Goes red against a bare `to_string()`.
#[test]
fn dust_amounts_stay_in_plain_decimal_notation() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds, bridge);

    assert_eq!(
        consumer.convert_smallest_to_human("1", 18).unwrap(),
        "0.000000000000000001"
    );
    assert_eq!(
        consumer.convert_smallest_to_human("123456789", 18).unwrap(),
        "0.000000000123456789"
    );
    // A whole amount keeps its short form rather than gaining the trailing
    // zeros the fixed scale would otherwise introduce.
    assert_eq!(
        consumer
            .convert_smallest_to_human("1000000000000000000", 18)
            .unwrap(),
        "1"
    );
}

/// A rate conversion must not emit more precision than the column that stores
/// it. `credited_amount` is `NUMERIC(78,18)`, so anything past the 18th
/// decimal is dropped on write - if the WebSocket carries the unrounded value,
/// it and the invoice API disagree from the 19th decimal onward.
#[test]
fn rate_conversion_is_rounded_to_the_stored_scale() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds, bridge);

    let converted = consumer
        .convert_payment_to_invoice_currency("500000000000000000", "2500.50", 18)
        .unwrap();

    let decimals = converted.split_once('.').map_or(0, |(_, frac)| frac.len());
    assert!(
        decimals <= 18,
        "amount carries {decimals} decimals, more than NUMERIC(78,18) stores: {converted}"
    );
}
