//! What payment detection *costs*, in RPC calls, as watched addresses grow.
//!
//! Every other test in this crate asks whether a payment was found. None asks
//! what finding it cost — and the answer decides the product's unit economics.
//!
//! Native detection polls one `eth_getBalance` per watched address per block.
//! On a 2-second chain that is ~1.3M requests per month for a *single* open
//! invoice, so a merchant with a handful of open invoices costs more in RPC
//! than a subscription would plausibly charge. ERC20 detection, in the same
//! monitor, puts every watched address into one `eth_getLogs` filter and costs
//! one call per block no matter how many addresses there are.
//!
//! That asymmetry is the finding. It is also the proof that O(1) per block is
//! achievable here rather than an inherent cost of watching a chain — one half
//! of this file already does it.
//!
//! These are regression tests for a cost characteristic, not for correctness,
//! and they are written to state the *shape* (calls grow with addresses, or do
//! not) rather than to pin a number an implementation could change innocently.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use evm::monitor::{
    ChainMonitor, ChainMonitorConfig, MockBlockSource, MonitorEvent, WatchedAddress, make_block,
};
use evm::{Address, U256};

const TEST_CHAIN_ID: u64 = 11155111;

fn test_chain_config() -> &'static evm::ChainConfig {
    evm::get_any_chain_config(TEST_CHAIN_ID).expect("Sepolia config exists")
}

fn test_monitor_config() -> ChainMonitorConfig {
    ChainMonitorConfig {
        required_confirmations: 3,
        max_blocks_per_scan: 100,
        confirmation_check_interval_secs: 60,
        stall_timeout_secs: 120,
        loop_hang_timeout_secs: 300,
        monitor_native: true,
        monitor_erc20: true,
    }
}

/// Start a monitor watching `n` addresses, feed it one block, and report how
/// many times each RPC method was called while processing it.
///
/// Counts are reset after startup and before the block is pushed, so what is
/// measured is the cost of *one block* rather than the cost of booting.
async fn calls_for_one_block(n: usize, token: Option<Address>) -> (u64, u64) {
    let source = MockBlockSource::new(TEST_CHAIN_ID);
    let probe = source.clone();

    let monitor = Arc::new(ChainMonitor::new(
        test_chain_config(),
        source,
        test_monitor_config(),
    ));

    for _ in 0..n {
        monitor
            .watch(WatchedAddress {
                address: Address::random(),
                invoice_id: uuid::Uuid::new_v4(),
                expected_amount: Some(U256::from(1u64)),
                token_contract: token,
                created_at: Utc::now(),
                last_known_balance: U256::ZERO,
            })
            .await;
    }

    let mut events = monitor.subscribe();
    let running = Arc::clone(&monitor);
    let handle = tokio::spawn(async move { running.start().await });

    // Wait for the monitor to be up before measuring, so startup's own calls
    // are not counted as the cost of a block.
    let started = tokio::time::timeout(Duration::from_secs(2), events.recv()).await;
    assert!(
        matches!(started, Ok(Ok(MonitorEvent::MonitorStarted { .. }))),
        "monitor did not start: {started:?}"
    );

    probe.reset_call_counts();
    probe.push_block(make_block(100));

    // No event is emitted for a block that contains no payment, so this waits
    // on the observable side effect instead: the calls the block produced.
    // Polled rather than slept on, so the test is as fast as the monitor is.
    for _ in 0..200 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        if probe.call_count("get_balance") >= n as u64 && probe.call_count("get_logs") >= 1 {
            break;
        }
        if token.is_some() && probe.call_count("get_logs") >= 1 {
            break;
        }
        if token.is_none() && probe.call_count("get_balance") >= n as u64 {
            break;
        }
    }

    let counts = (
        probe.call_count("get_balance"),
        probe.call_count("get_logs"),
    );
    handle.abort();
    counts
}

/// Native detection costs **two** `eth_getBalance` calls per watched address,
/// per block.
///
/// Two, not one, and the second is redundant:
///
/// * `check_native_payments` reads each address's balance at `block.number`
///   to see whether it went up.
/// * `update_watched_balances` then reads *the same address at the same
///   block* again, to store it as `last_known_balance` — a value the first
///   read already had in hand.
///
/// So half of the dominant cost in this monitor is a value being fetched
/// twice. That is separable from the larger reshaping (read the block once,
/// match locally) and worth taking on its own.
///
/// Pinned at exactly `2 * n` deliberately. This is the number the cost model
/// is built on, so it should not be free to drift in either direction without
/// someone noticing: if it grows, cost grew; if it shrinks, the fix landed and
/// this test should be rewritten to pin the new shape rather than deleted.
#[tokio::test]
async fn native_detection_costs_two_calls_per_watched_address_per_block() {
    let mut measured = Vec::new();
    for n in [1usize, 2, 4, 8] {
        let (balance_calls, _) = calls_for_one_block(n, None).await;
        measured.push((n, balance_calls));
    }

    for (n, calls) in &measured {
        assert_eq!(
            *calls,
            2 * *n as u64,
            "one block with {n} watched native addresses costs {} get_balance calls \
             (one to detect, one to re-read what was just detected). Measured across \
             sizes: {measured:?}",
            2 * n
        );
    }

    // States the shape rather than only the coefficient: if this ever becomes
    // independent of `n`, the rework landed.
    let (first_n, first_calls) = measured[0];
    let (last_n, last_calls) = measured[measured.len() - 1];
    assert!(
        last_calls > first_calls,
        "native detection is expected to scale with watched addresses today \
         ({first_n} -> {first_calls}, {last_n} -> {last_calls}); if it no longer does, \
         rewrite this test to pin the new shape"
    );
}

/// ERC20 detection: exactly one `eth_getLogs` per block, at any scale.
///
/// The contrast is the point. Both halves run in the same `process_block`,
/// over the same watched set, on the same chain — so the difference is the
/// query shape and nothing else.
#[tokio::test]
async fn erc20_detection_costs_one_call_per_block_at_any_scale() {
    let token = Address::random();
    let mut measured = Vec::new();
    for n in [1usize, 8, 64] {
        let (_, log_calls) = calls_for_one_block(n, Some(token)).await;
        measured.push((n, log_calls));
    }

    for (n, calls) in &measured {
        assert_eq!(
            *calls, 1,
            "one block should cost exactly one get_logs regardless of how many \
             addresses are watched; {n} addresses produced {calls}. Measured: {measured:?}"
        );
    }
}
