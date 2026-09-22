//! The tokens seeded for zkSync Era, Linea, Scroll and the Sepolia-family
//! testnets, resolved the way a real payment resolves them.
//!
//! A row nothing looks up is the same bug as no row at all - `payment_handler`
//! calls `TokenReader::get_by_address` with a lowercase address it read off
//! the chain, so a seed row that only matches its own checksummed casing back
//! would still leave every real payment showing a truncated address.

use types::{ChainId, TokenReader};

use super::create_test_service;

#[tokio::test]
#[ignore]
async fn zksync_era_usdc_resolves_by_lowercased_address() {
    let Some(service) = create_test_service().await else {
        return;
    };

    let token = TokenReader::get_by_address(
        &service,
        &ChainId::evm(324),
        // Lowercased, as the monitor formats an address it read off-chain -
        // the seed row itself is checksummed.
        "0x1d17cbcf0d6d143135ae902365d2e5e2a16538d4",
    )
    .await
    .expect("query succeeds")
    .expect("zkSync Era USDC is seeded");

    assert_eq!(token.symbol.as_deref(), Some("USDC"));
    assert_eq!(token.decimals, Some(6));
}

#[tokio::test]
#[ignore]
async fn optimism_sepolia_usdc_resolves() {
    let Some(service) = create_test_service().await else {
        return;
    };

    let token = TokenReader::get_by_address(
        &service,
        &ChainId::evm(11_155_420),
        "0x5fd84259d66cd46123540766be93dfe6d43130d7",
    )
    .await
    .expect("query succeeds")
    .expect("Optimism Sepolia USDC is seeded");

    assert_eq!(token.symbol.as_deref(), Some("USDC"));
    assert_eq!(token.decimals, Some(6));
}

#[tokio::test]
#[ignore]
async fn linea_usdc_resolves_by_lowercased_address() {
    let Some(service) = create_test_service().await else {
        return;
    };

    let token = TokenReader::get_by_address(
        &service,
        &ChainId::evm(59_144),
        "0x176211869ca2b568f2a7d4ee941e073a821ee1ff",
    )
    .await
    .expect("query succeeds")
    .expect("Linea USDC is seeded");

    assert_eq!(token.symbol.as_deref(), Some("USDC"));
    assert_eq!(token.decimals, Some(6));
}

#[tokio::test]
#[ignore]
async fn scroll_weth_decimals_round_trip_to_the_right_display_amount() {
    let Some(service) = create_test_service().await else {
        return;
    };

    let token = TokenReader::get_by_address(
        &service,
        &ChainId::evm(534_352),
        "0x5300000000000000000000000000000000000004",
    )
    .await
    .expect("query succeeds")
    .expect("Scroll WETH is seeded");

    assert_eq!(token.symbol.as_deref(), Some("WETH"));
    // 18 decimals: 1_500_000_000_000_000_000 base units is 1.5 WETH, not
    // 1.5e12 - the bug this repository already got bitten by once, on BNB
    // Chain's 18-decimal USDC/USDT.
    let base_units: u128 = 1_500_000_000_000_000_000;
    let display = base_units as f64 / 10f64.powi(i32::from(token.decimals.unwrap()));
    assert_eq!(display, 1.5);
}
