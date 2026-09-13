//! Integration tests for the payment detection pipeline.
//!
//! Tests the full ChainMonitor flow: watch address, inject synthetic blocks,
//! verify PaymentDetected and PaymentConfirmed events are emitted correctly.

use std::sync::Arc;
use std::time::Duration;

use evm::monitor::{
    ChainMonitor, ChainMonitorConfig, MockBlockSource, MonitorEvent, WatchedAddress, make_block,
    make_block_with_parent, make_erc20_transfer_log, make_native_transfer,
};
use evm::{Address, B256, U256};

use chrono::Utc;

/// Sepolia chain ID.
const TEST_CHAIN_ID: u64 = 11155111;

fn test_chain_config() -> &'static evm::ChainConfig {
    evm::get_any_chain_config(TEST_CHAIN_ID).expect("Sepolia config exists")
}

fn test_monitor_config(required_confirmations: u64) -> ChainMonitorConfig {
    ChainMonitorConfig {
        required_confirmations,
        max_blocks_per_scan: 100,
        // Short interval so confirmation checks happen quickly in tests
        confirmation_check_interval_secs: 1,
        monitor_native: true,
        monitor_erc20: true,
    }
}

// ============================================================================
// Native ETH payment: detection + confirmation
// ============================================================================

#[tokio::test]
#[allow(clippy::too_many_lines)] // end-to-end payment flow test with multi-step setup + assertions
async fn test_native_payment_detection_and_confirmation() {
    let source = MockBlockSource::new(TEST_CHAIN_ID);
    let test_source = source.clone(); // shared handle for injection

    let payment_address = Address::random();
    let sender = Address::random();
    let invoice_id = uuid::Uuid::new_v4();
    let payment_amount = U256::from(50_000_000_000_000_000u64); // 0.05 ETH
    let tx_hash = B256::random();

    let monitor = Arc::new(ChainMonitor::new(
        test_chain_config(),
        source,
        test_monitor_config(3),
    ));

    // Watch the address for native ETH
    monitor
        .watch(WatchedAddress {
            address: payment_address,
            invoice_id,
            expected_amount: Some(payment_amount),
            token_contract: None,
            created_at: Utc::now(),
            last_known_balance: U256::ZERO,
        })
        .await;

    let mut event_rx = monitor.subscribe();

    // Start monitor
    let monitor_clone = monitor.clone();
    let monitor_handle = tokio::spawn(async move { monitor_clone.start().await });

    // Wait for MonitorStarted
    let started = tokio::time::timeout(Duration::from_secs(2), event_rx.recv()).await;
    assert!(
        matches!(started, Ok(Ok(MonitorEvent::MonitorStarted { .. }))),
        "expected MonitorStarted"
    );

    // Simulate payment: set new balance and add the transfer
    test_source
        .set_balance(payment_address, payment_amount)
        .await;
    test_source
        .add_native_transfer(
            100,
            make_native_transfer(sender, payment_address, payment_amount, tx_hash),
        )
        .await;

    // Push block 100
    test_source.push_block(make_block(100));

    // Expect PaymentDetected
    let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .expect("timeout waiting for event")
        .expect("channel error");

    match event {
        MonitorEvent::PaymentDetected(p) => {
            assert_eq!(p.chain_id, TEST_CHAIN_ID);
            assert_eq!(p.invoice_id, invoice_id);
            assert_eq!(p.payment_address, payment_address);
            assert_eq!(p.amount, payment_amount);
            assert_eq!(p.tx_hash, tx_hash);
            assert_eq!(p.block_number, 100);
            assert!(p.is_native);
            assert!(p.token_address.is_none());
            assert_eq!(p.from_address, sender);
            assert_eq!(p.confirmations, 1);
            assert_eq!(p.required_confirmations, 3);
        }
        other => panic!("expected PaymentDetected, got {:?}", other),
    }

    // Advance blocks to trigger confirmation (need 3 confirmations: blocks 100, 101, 102)
    // Block 101 (2 confirmations)
    test_source.push_block(make_block(101));
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Block 102 (3 confirmations) — should trigger PaymentConfirmed
    test_source.push_block(make_block(102));

    // Wait for confirmation check to fire (up to 2 seconds)
    let mut confirmed = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), event_rx.recv()).await {
            Ok(Ok(MonitorEvent::PaymentConfirmed(c))) => {
                assert_eq!(c.chain_id, TEST_CHAIN_ID);
                assert_eq!(c.invoice_id, invoice_id);
                assert_eq!(c.tx_hash, tx_hash);
                assert_eq!(c.amount, payment_amount);
                assert!(c.confirmations >= 3);
                confirmed = true;
                break;
            }
            Ok(Ok(_)) => continue, // skip other events
            _ => continue,
        }
    }
    assert!(confirmed, "expected PaymentConfirmed within timeout");

    monitor.stop().await.unwrap();
    let _ = monitor_handle.await;
}

// ============================================================================
// ERC20 token payment detection
// ============================================================================

#[tokio::test]
async fn test_erc20_payment_detection() {
    let source = MockBlockSource::new(TEST_CHAIN_ID);
    let test_source = source.clone();

    let payment_address = Address::random();
    let sender = Address::random();
    let token_contract = Address::random();
    let invoice_id = uuid::Uuid::new_v4();
    let payment_amount = U256::from(100_000_000u64); // 100 USDT (6 decimals)
    let tx_hash = B256::random();

    let monitor = Arc::new(ChainMonitor::new(
        test_chain_config(),
        source,
        test_monitor_config(3),
    ));

    // Watch address for ERC20 token (token_contract = Some)
    monitor
        .watch(WatchedAddress {
            address: payment_address,
            invoice_id,
            expected_amount: Some(payment_amount),
            token_contract: Some(token_contract),
            created_at: Utc::now(),
            last_known_balance: U256::ZERO,
        })
        .await;

    let mut event_rx = monitor.subscribe();

    let monitor_clone = monitor.clone();
    let monitor_handle = tokio::spawn(async move { monitor_clone.start().await });

    // Wait for MonitorStarted
    let _ = tokio::time::timeout(Duration::from_secs(2), event_rx.recv()).await;

    // Inject ERC20 Transfer log for block 200
    test_source
        .add_log(
            200,
            make_erc20_transfer_log(
                token_contract,
                sender,
                payment_address,
                payment_amount,
                200,
                tx_hash,
                0,
            ),
        )
        .await;

    // Push block 200
    test_source.push_block(make_block(200));

    // Expect PaymentDetected
    let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .expect("timeout waiting for event")
        .expect("channel error");

    match event {
        MonitorEvent::PaymentDetected(p) => {
            assert_eq!(p.chain_id, TEST_CHAIN_ID);
            assert_eq!(p.invoice_id, invoice_id);
            assert_eq!(p.payment_address, payment_address);
            assert_eq!(p.amount, payment_amount);
            assert_eq!(p.tx_hash, tx_hash);
            assert!(!p.is_native);
            assert_eq!(p.token_address, Some(token_contract));
            assert_eq!(p.from_address, sender);
            assert_eq!(p.log_index, Some(0));
        }
        other => panic!("expected PaymentDetected, got {:?}", other),
    }

    monitor.stop().await.unwrap();
    let _ = monitor_handle.await;
}

// ============================================================================
// Underpayment: partial payment keeps pending, second completes
// ============================================================================

#[tokio::test]
async fn test_underpayment_two_transactions() {
    let source = MockBlockSource::new(TEST_CHAIN_ID);
    let test_source = source.clone();

    let payment_address = Address::random();
    let sender = Address::random();
    let invoice_id = uuid::Uuid::new_v4();
    let half_amount = U256::from(25_000_000_000_000_000u64); // 0.025 ETH
    let tx_hash_1 = B256::random();
    let tx_hash_2 = B256::random();

    let monitor = Arc::new(ChainMonitor::new(
        test_chain_config(),
        source,
        test_monitor_config(1), // 1 confirmation for simplicity
    ));

    monitor
        .watch(WatchedAddress {
            address: payment_address,
            invoice_id,
            expected_amount: Some(half_amount * U256::from(2)),
            token_contract: None,
            created_at: Utc::now(),
            last_known_balance: U256::ZERO,
        })
        .await;

    let mut event_rx = monitor.subscribe();
    let monitor_clone = monitor.clone();
    let monitor_handle = tokio::spawn(async move { monitor_clone.start().await });

    // Wait for start
    let _ = tokio::time::timeout(Duration::from_secs(2), event_rx.recv()).await;

    // First partial payment
    test_source.set_balance(payment_address, half_amount).await;
    test_source
        .add_native_transfer(
            100,
            make_native_transfer(sender, payment_address, half_amount, tx_hash_1),
        )
        .await;
    test_source.push_block(make_block(100));

    let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .expect("timeout")
        .expect("channel error");
    match &event {
        MonitorEvent::PaymentDetected(p) => {
            assert_eq!(p.amount, half_amount);
            assert_eq!(p.tx_hash, tx_hash_1);
        }
        other => panic!("expected first PaymentDetected, got {:?}", other),
    }

    // Second partial payment (completes the full amount)
    let full_amount = half_amount * U256::from(2);
    test_source.set_balance(payment_address, full_amount).await;
    test_source
        .add_native_transfer(
            101,
            make_native_transfer(sender, payment_address, half_amount, tx_hash_2),
        )
        .await;
    test_source.push_block(make_block(101));

    // Should get second PaymentDetected
    let mut got_second = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), event_rx.recv()).await {
            Ok(Ok(MonitorEvent::PaymentDetected(p))) if p.tx_hash == tx_hash_2 => {
                assert_eq!(p.amount, half_amount);
                got_second = true;
                break;
            }
            Ok(Ok(_)) => continue,
            _ => continue,
        }
    }
    assert!(got_second, "expected second PaymentDetected");

    monitor.stop().await.unwrap();
    let _ = monitor_handle.await;
}

// ============================================================================
// No watched addresses = no events
// ============================================================================

#[tokio::test]
async fn test_no_watched_addresses_no_events() {
    let source = MockBlockSource::new(TEST_CHAIN_ID);
    let test_source = source.clone();

    let monitor = Arc::new(ChainMonitor::new(
        test_chain_config(),
        source,
        test_monitor_config(3),
    ));

    let mut event_rx = monitor.subscribe();
    let monitor_clone = monitor.clone();
    let monitor_handle = tokio::spawn(async move { monitor_clone.start().await });

    // Wait for start
    let _ = tokio::time::timeout(Duration::from_secs(2), event_rx.recv()).await;

    // Push blocks with no watches
    test_source.push_block(make_block(100));
    test_source.push_block(make_block(101));

    // Should not receive any payment events
    let result = tokio::time::timeout(Duration::from_millis(500), event_rx.recv()).await;
    if let Ok(Ok(MonitorEvent::PaymentDetected(_))) = result {
        panic!("should not detect payment with no watches")
    }

    monitor.stop().await.unwrap();
    let _ = monitor_handle.await;
}

// ============================================================================
// Reorg detection and re-validation (RCS-295)
// ============================================================================

/// Wait for the next `ReorgDetected` event, skipping anything else.
async fn wait_for_reorg(
    event_rx: &mut tokio::sync::broadcast::Receiver<MonitorEvent>,
) -> evm::monitor::events::ReorgDetected {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), event_rx.recv()).await {
            Ok(Ok(MonitorEvent::ReorgDetected(r))) => return r,
            Ok(Ok(_)) => continue,
            _ => continue,
        }
    }
    panic!("timed out waiting for ReorgDetected");
}

/// A fork arriving more than one block ahead of the last processed block used
/// to go unnoticed entirely: `process_block` only ever compared `parent_hash`
/// when the new block was the immediate successor. Here block 103 arrives
/// right after block 100, skipping 101 and 102, so that direct comparison
/// never runs — detection must fall back to asking the chain whether block
/// 100 is still canonical.
#[tokio::test]
async fn test_reorg_detected_across_a_block_gap() {
    let source = MockBlockSource::new(TEST_CHAIN_ID);
    let test_source = source.clone();

    let monitor = Arc::new(ChainMonitor::new(
        test_chain_config(),
        source,
        test_monitor_config(3),
    ));

    let mut event_rx = monitor.subscribe();
    let monitor_clone = monitor.clone();
    let monitor_handle = tokio::spawn(async move { monitor_clone.start().await });
    let _ = tokio::time::timeout(Duration::from_secs(2), event_rx.recv()).await;

    test_source.push_block(make_block(100));
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Block 100 was reorged out from under us, but the next notification
    // skips straight to block 103.
    test_source.set_block_hash(100, B256::random());
    test_source.push_block(make_block(103));

    let reorg = wait_for_reorg(&mut event_rx).await;
    assert_eq!(reorg.fork_block, 100);

    monitor.stop().await.unwrap();
    let _ = monitor_handle.await;
}

/// A transaction that survives a reorg by landing in a different block must
/// be reported as survived, not treated as gone. Retracting it anyway is the
/// opposite error: it un-pays an invoice that a customer genuinely settled.
/// This is the case a naive "everything at or above the fork block is gone"
/// fix gets wrong.
#[tokio::test]
async fn test_reorg_reports_a_relocated_transaction_as_survived() {
    let source = MockBlockSource::new(TEST_CHAIN_ID);
    let test_source = source.clone();

    let payment_address = Address::random();
    let sender = Address::random();
    let invoice_id = uuid::Uuid::new_v4();
    let payment_amount = U256::from(50_000_000_000_000_000u64);
    let tx_hash = B256::random();

    let monitor = Arc::new(ChainMonitor::new(
        test_chain_config(),
        source,
        test_monitor_config(3),
    ));

    monitor
        .watch(WatchedAddress {
            address: payment_address,
            invoice_id,
            expected_amount: Some(payment_amount),
            token_contract: None,
            created_at: Utc::now(),
            last_known_balance: U256::ZERO,
        })
        .await;

    let mut event_rx = monitor.subscribe();
    let monitor_clone = monitor.clone();
    let monitor_handle = tokio::spawn(async move { monitor_clone.start().await });
    let _ = tokio::time::timeout(Duration::from_secs(2), event_rx.recv()).await;

    // The payment is first detected at block 100.
    test_source
        .set_balance(payment_address, payment_amount)
        .await;
    test_source
        .add_native_transfer(
            100,
            make_native_transfer(sender, payment_address, payment_amount, tx_hash),
        )
        .await;
    test_source.push_block(make_block(100));

    let detected = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .expect("timeout waiting for PaymentDetected")
        .expect("channel error");
    assert!(matches!(detected, MonitorEvent::PaymentDetected(_)));

    // A reorg replaces block 100, but the very same transaction lands again
    // at block 101 on the new canonical chain rather than disappearing.
    test_source
        .add_native_transfer(
            101,
            make_native_transfer(sender, payment_address, payment_amount, tx_hash),
        )
        .await;
    test_source.push_block(make_block_with_parent(101, B256::random(), B256::random()));

    let reorg = wait_for_reorg(&mut event_rx).await;
    assert_eq!(reorg.fork_block, 100);
    assert!(
        reorg.survived_tx_hashes.contains(&tx_hash),
        "a transaction relocated to a different block must be reported as survived, not gone"
    );

    monitor.stop().await.unwrap();
    let _ = monitor_handle.await;
}

/// A reorg window wider than `max_blocks_per_scan` must still find a
/// relocated transaction anywhere in `[fork_block, new_head]`, not just in
/// the tail nearest the new head. Clamping the re-scan window to that knob —
/// the same one ordinary block processing uses to bound history scans —
/// silently drops survivors below the clamp and retracts them anyway: the
/// opposite error, on a wider reorg than the single-block case above
/// exercises.
#[tokio::test]
async fn test_reorg_wider_than_scan_cap_still_finds_a_relocated_transaction() {
    let source = MockBlockSource::new(TEST_CHAIN_ID);
    let test_source = source.clone();

    let payment_address = Address::random();
    let sender = Address::random();
    let invoice_id = uuid::Uuid::new_v4();
    let payment_amount = U256::from(50_000_000_000_000_000u64);
    let tx_hash = B256::random();

    let mut config = test_monitor_config(3);
    config.max_blocks_per_scan = 2;

    let monitor = Arc::new(ChainMonitor::new(test_chain_config(), source, config));

    monitor
        .watch(WatchedAddress {
            address: payment_address,
            invoice_id,
            expected_amount: Some(payment_amount),
            token_contract: None,
            created_at: Utc::now(),
            last_known_balance: U256::ZERO,
        })
        .await;

    let mut event_rx = monitor.subscribe();
    let monitor_clone = monitor.clone();
    let monitor_handle = tokio::spawn(async move { monitor_clone.start().await });
    let _ = tokio::time::timeout(Duration::from_secs(2), event_rx.recv()).await;

    test_source.push_block(make_block(100));
    tokio::time::sleep(Duration::from_millis(100)).await;

    // The transaction relocates to block 105 — far below the last
    // `max_blocks_per_scan` (2) blocks of the eventual [100, 130] window.
    test_source
        .add_native_transfer(
            105,
            make_native_transfer(sender, payment_address, payment_amount, tx_hash),
        )
        .await;

    // Block 100 is reorged out, and the chain jumps straight to block 130 —
    // a fork window far wider than `max_blocks_per_scan`.
    test_source.set_block_hash(100, B256::random());
    test_source.push_block(make_block(130));

    let reorg = wait_for_reorg(&mut event_rx).await;
    assert_eq!(reorg.fork_block, 100);
    assert!(
        reorg.survived_tx_hashes.contains(&tx_hash),
        "a transaction relocated below the scan cap's tail must still be found, not skipped"
    );

    monitor.stop().await.unwrap();
    let _ = monitor_handle.await;
}
