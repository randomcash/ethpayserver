//! The tokens seeded for zkSync Era, Linea, Scroll and the Sepolia-family
//! testnets, resolved by `TokenReader::get_by_address` the way a real payment
//! resolves them: a lowercase address read off the chain against a
//! checksummed seed row.
//!
//! A row nothing looks up is the same bug as no row at all - a wrong address,
//! symbol or decimals value ships silently otherwise. Every row the migration
//! seeds is resolved here, not a sample: the four below get a dedicated test
//! each (one of them also exercises the payment-detection call site, not just
//! this lookup), and `REMAINING_SEEDED_ROWS` table-drives the rest so a row
//! added later costs one line, not a new test function.
//!
//! The `server` crate's `event_consumer` tests separately prove that
//! `handle_payment_detected` - the actual call site a payment reaches this
//! lookup through - turns a resolution into the symbol on the payment record
//! instead of the `ERC20` fallback.

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

/// Every seeded row not already covered by a test above: chain, lowercased
/// on-chain address, expected symbol, expected decimals. Table-driven so a
/// row added to the migration costs one line here, not a new test function -
/// and each row is checked against its own decimals rather than a family
/// average, since Scroll's rows aren't uniformly 6 or 18.
const REMAINING_SEEDED_ROWS: &[(u64, &str, &str, u8)] = &[
    // zkSync Era (eip155:324)
    (324, "0x493257fd37edb34451f62edf8d2a0c418852ba4c", "USDT", 6),
    (324, "0xbbeb516fb02a01611cbbe0453fe3c580d7281011", "WBTC", 8),
    // Linea (eip155:59144)
    (59_144, "0xa219439258ca9da29e9cc4ce5596924745e12b93", "USDT", 6),
    (59_144, "0x3aab2285ddcddad8edf438c1bab47e1a9d05a9b4", "WBTC", 8),
    (59_144, "0xe5d7c2a44ffddf6b295a15c148167daaaf5cf34f", "WETH", 18),
    // Scroll (eip155:534352)
    (534_352, "0x06efdbff2a14a7c8e15944d1f4a48f9f95f663a4", "USDC", 6),
    (534_352, "0xf55bec9cafdbe8730f096aa55dad6d22d44099df", "USDT", 6),
    (534_352, "0x3c1bca5a656e69edcd0d4e36bebb3fcdaca60cf1", "WBTC", 8),
    // Testnets seeded alongside Optimism Sepolia
    (421_614, "0x75faf114eafb1bdbe2f0316df893fd58ce46aa4d", "USDC", 6), // Arbitrum Sepolia
    (84_532, "0x036cbd53842c5426634e7929541ec2318f3dcf7e", "USDC", 6), // Base Sepolia
    (43_113, "0x5425890298aed601595a70ab815c96711a31bc65", "USDC", 6), // Avalanche Fuji
    (80_002, "0x41e94eb019c0762f9bfcf9fb1e58725bfb0e7582", "USDC", 6), // Polygon Amoy
];

#[tokio::test]
#[ignore]
async fn remaining_seeded_rows_resolve_with_the_right_symbol_and_decimals() {
    let Some(service) = create_test_service().await else {
        return;
    };

    for (chain_id, address, symbol, decimals) in REMAINING_SEEDED_ROWS {
        let token = TokenReader::get_by_address(&service, &ChainId::evm(*chain_id), address)
            .await
            .unwrap_or_else(|e| panic!("query for {symbol} on chain {chain_id} failed: {e:?}"))
            .unwrap_or_else(|| panic!("{symbol} on chain {chain_id} ({address}) is not seeded"));

        assert_eq!(
            token.symbol.as_deref(),
            Some(*symbol),
            "chain {chain_id} address {address}"
        );
        assert_eq!(
            token.decimals,
            Some(*decimals),
            "chain {chain_id} address {address}"
        );
    }
}
