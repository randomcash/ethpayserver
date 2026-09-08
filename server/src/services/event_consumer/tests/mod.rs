mod helpers;
mod payment_confirmed;
mod payment_detected;
mod reorg;

use super::*;
use helpers::native_symbol;

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
