mod helpers;
mod payment_confirmed;
mod payment_detected;
mod reorg;

use super::*;
use helpers::network_native_symbol;

#[test]
fn test_network_native_symbol() {
    use types::Network;
    assert_eq!(network_native_symbol(Network::Ethereum), "ETH");
    assert_eq!(network_native_symbol(Network::Polygon), "POL");
    assert_eq!(network_native_symbol(Network::Avalanche), "AVAX");
    assert_eq!(network_native_symbol(Network::BinanceSmartChain), "BNB");
    assert_eq!(network_native_symbol(Network::Arbitrum), "ETH");
    assert_eq!(network_native_symbol(Network::Optimism), "ETH");
    assert_eq!(network_native_symbol(Network::Base), "ETH");
    assert_eq!(network_native_symbol(Network::Fantom), "FTM");
    assert_eq!(network_native_symbol(Network::Gnosis), "xDAI");
    // Non-EVM networks
    assert_eq!(network_native_symbol(Network::BitcoinMainnet), "UNKNOWN");
}

#[test]
fn test_event_consumer_error_display() {
    let db_err = EventConsumerError::Database(types::RepositoryError::NotFound("test".into()));
    assert!(db_err.to_string().contains("database error"));

    let data_err = EventConsumerError::InvalidData("bad data".into());
    assert!(data_err.to_string().contains("invalid data"));
}
