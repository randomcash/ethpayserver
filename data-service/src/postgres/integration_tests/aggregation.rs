//! Payment aggregation E2E tests.
//!
//! These tests verify that payments in different currencies/chains are properly
//! aggregated into a single invoice total using the `credited_amount` field.

use std::collections::HashSet;

use types::{
    InvoiceReader, InvoiceStatus, InvoiceWriter, PaymentOptionWriter, PaymentReader, PaymentWriter,
};

use super::{
    create_test_service, test_invoice, test_payment_option_with_rate, test_payment_with_credit,
};

/// E2E test: Multi-currency payment aggregation
///
/// Scenario:
/// - Create a $100 USD invoice
/// - Payment option 1: ETH at rate 0.0005 ETH/USD (pay 0.05 ETH for $100)
/// - Payment option 2: USDC at rate 1.0 USDC/USD (pay 100 USDC for $100)
/// - User pays $60 worth of ETH (0.03 ETH)
/// - User pays $40 worth of USDC (40 USDC)
/// - Verify: amount_received = $100
#[tokio::test]
#[ignore]
async fn integration_multi_currency_payment_aggregation() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create a $100 USD invoice
    let mut invoice = test_invoice();
    invoice.currency = "USD".to_string();
    invoice.amount = "100".to_string();
    invoice.amount_received = "0".to_string();
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // Create ETH payment option (rate: 1 USD = 0.0005 ETH)
    let eth_po = test_payment_option_with_rate(
        &invoice.id,
        1, // Ethereum mainnet
        "ETH",
        None,                       // Native asset
        18,                         // ETH decimals
        "50000000000000000",        // 0.05 ETH in wei (for $100)
        Some("0.0005".to_string()), // rate
    );
    PaymentOptionWriter::create(&service, &eth_po)
        .await
        .unwrap();

    // Create USDC payment option (rate: 1 USD = 1 USDC)
    let usdc_po = test_payment_option_with_rate(
        &invoice.id,
        1, // Ethereum mainnet
        "USDC",
        Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string()),
        6,                     // USDC decimals
        "100000000",           // 100 USDC (6 decimals) for $100
        Some("1".to_string()), // rate
    );
    PaymentOptionWriter::create(&service, &usdc_po)
        .await
        .unwrap();

    // Payment 1: User pays $60 worth of ETH
    // 0.03 ETH = 30000000000000000 wei
    // credited_amount = (30000000000000000 / 10^18) / 0.0005 = 0.03 / 0.0005 = 60 USD
    let eth_payment = test_payment_with_credit(
        &invoice.id,
        Some(eth_po.id.0),
        1,
        "ETH",
        "30000000000000000",
        Some("60".to_string()),
        Some("0.0005".to_string()),
    );
    PaymentWriter::upsert(&service, &eth_payment).await.unwrap();

    // Check intermediate state: amount_received should be $60
    let fetched = InvoiceReader::get(&service, &invoice.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fetched.amount_received, "60",
        "After ETH payment, amount_received should be $60"
    );

    // Payment 2: User pays $40 worth of USDC
    // 40 USDC = 40000000 (6 decimals)
    // credited_amount = (40000000 / 10^6) / 1 = 40 / 1 = 40 USD
    let usdc_payment = test_payment_with_credit(
        &invoice.id,
        Some(usdc_po.id.0),
        1,
        "USDC",
        "40000000",
        Some("40".to_string()),
        Some("1".to_string()),
    );
    PaymentWriter::upsert(&service, &usdc_payment)
        .await
        .unwrap();

    // Check final state: amount_received should be $100 (60 + 40)
    let fetched = InvoiceReader::get(&service, &invoice.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fetched.amount_received, "100",
        "After both payments, amount_received should be $100"
    );

    // Verify invoice transitioned to processing (trigger sets this on first payment)
    assert_eq!(
        fetched.status,
        InvoiceStatus::Processing,
        "Invoice should be in Processing status after receiving payments"
    );

    // Verify both payments are recorded
    let payments = PaymentReader::get_for_invoice(&service, &invoice.id)
        .await
        .unwrap();
    assert_eq!(payments.len(), 2, "Should have 2 payments recorded");

    // Verify each payment has correct credited_amount
    let eth_p = payments.iter().find(|p| p.asset_symbol == "ETH").unwrap();
    assert_eq!(eth_p.credited_amount, Some("60".to_string()));

    let usdc_p = payments.iter().find(|p| p.asset_symbol == "USDC").unwrap();
    assert_eq!(usdc_p.credited_amount, Some("40".to_string()));
}

/// E2E test: Same-asset payment (no rate conversion)
///
/// Scenario:
/// - Create a 1.5 ETH invoice
/// - User pays 1.5 ETH directly
/// - No rate conversion needed, credited_amount = human-readable amount
#[tokio::test]
#[ignore]
async fn integration_same_asset_payment_no_conversion() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create a 1.5 ETH invoice
    let mut invoice = test_invoice();
    invoice.currency = "ETH".to_string();
    invoice.amount = "1.5".to_string();
    invoice.amount_received = "0".to_string();
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // Create ETH payment option (no rate - same asset)
    let eth_po = test_payment_option_with_rate(
        &invoice.id,
        1,
        "ETH",
        None,
        18,
        "1500000000000000000",
        None, // No rate - same asset
    );
    PaymentOptionWriter::create(&service, &eth_po)
        .await
        .unwrap();

    // User pays 1.5 ETH
    // credited_amount = 1500000000000000000 / 10^18 = 1.5 ETH
    let payment = test_payment_with_credit(
        &invoice.id,
        Some(eth_po.id.0),
        1,
        "ETH",
        "1500000000000000000",
        Some("1.5".to_string()),
        None,
    );
    PaymentWriter::upsert(&service, &payment).await.unwrap();

    // Verify amount_received matches
    let fetched = InvoiceReader::get(&service, &invoice.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fetched.amount_received, "1.5",
        "amount_received should be 1.5 ETH"
    );
}

/// E2E test: Payment without credited_amount is not counted
///
/// Scenario:
/// - Create a $100 USD invoice
/// - Payment arrives but conversion fails (credited_amount = None)
/// - Verify: payment is recorded but amount_received stays at 0
#[tokio::test]
#[ignore]
async fn integration_payment_without_credit_not_counted() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create a $100 USD invoice
    let mut invoice = test_invoice();
    invoice.currency = "USD".to_string();
    invoice.amount = "100".to_string();
    invoice.amount_received = "0".to_string();
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // Payment arrives but credited_amount is None (conversion failed)
    let payment = test_payment_with_credit(
        &invoice.id,
        None,
        1,
        "ETH",
        "50000000000000000",
        None, // No credited_amount - conversion failed
        None,
    );
    PaymentWriter::upsert(&service, &payment).await.unwrap();

    // Verify payment is recorded
    let payments = PaymentReader::get_for_invoice(&service, &invoice.id)
        .await
        .unwrap();
    assert_eq!(payments.len(), 1, "Payment should be recorded");

    // But amount_received should still be 0 (payment not counted)
    let fetched = InvoiceReader::get(&service, &invoice.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fetched.amount_received, "0",
        "Payment without credited_amount should not be counted"
    );

    // Invoice should still transition to processing (payment was detected)
    assert_eq!(fetched.status, InvoiceStatus::Processing);
}

/// E2E test: Reorged payment is excluded from aggregation
///
/// Scenario:
/// - Create a $100 USD invoice
/// - Two payments: $60 + $50 = $110
/// - First payment gets reorged
/// - Verify: amount_received = $50 (only non-reorged payment counted)
#[tokio::test]
#[ignore]
async fn integration_reorged_payment_excluded_from_aggregation() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create a $100 USD invoice
    let mut invoice = test_invoice();
    invoice.currency = "USD".to_string();
    invoice.amount = "100".to_string();
    invoice.amount_received = "0".to_string();
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // Create payment option
    let eth_po = test_payment_option_with_rate(
        &invoice.id,
        1,
        "ETH",
        None,
        18,
        "50000000000000000",
        Some("0.0005".to_string()),
    );
    PaymentOptionWriter::create(&service, &eth_po)
        .await
        .unwrap();

    // Payment 1: $60
    let payment1 = test_payment_with_credit(
        &invoice.id,
        Some(eth_po.id.0),
        1,
        "ETH",
        "30000000000000000",
        Some("60".to_string()),
        Some("0.0005".to_string()),
    );
    PaymentWriter::upsert(&service, &payment1).await.unwrap();

    // Payment 2: $50
    let payment2 = test_payment_with_credit(
        &invoice.id,
        Some(eth_po.id.0),
        1,
        "ETH",
        "25000000000000000",
        Some("50".to_string()),
        Some("0.0005".to_string()),
    );
    PaymentWriter::upsert(&service, &payment2).await.unwrap();

    // Verify total is $110
    let fetched = InvoiceReader::get(&service, &invoice.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.amount_received, "110");

    // Reorg: payment1 gets invalidated
    PaymentWriter::mark_reorged(&service, &invoice.id, 1, payment1.block_number.unwrap())
        .await
        .unwrap();

    // Verify amount_received is now only $50
    let fetched = InvoiceReader::get(&service, &invoice.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fetched.amount_received, "50",
        "Reorged payment should not be counted"
    );
}

/// E2E test: Multi-chain payment aggregation
///
/// Scenario:
/// - Create a $100 USD invoice
/// - Payment option on Ethereum (chain 1) and Polygon (chain 137)
/// - User pays $50 on Ethereum, $50 on Polygon
/// - Verify: amount_received = $100
#[tokio::test]
#[ignore]
async fn integration_multi_chain_payment_aggregation() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create a $100 USD invoice
    let mut invoice = test_invoice();
    invoice.currency = "USD".to_string();
    invoice.amount = "100".to_string();
    invoice.amount_received = "0".to_string();
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // ETH payment option on Ethereum mainnet
    let eth_po = test_payment_option_with_rate(
        &invoice.id,
        1,
        "ETH",
        None,
        18,
        "50000000000000000",
        Some("0.0005".to_string()),
    );
    PaymentOptionWriter::create(&service, &eth_po)
        .await
        .unwrap();

    // POL payment option on Polygon
    let pol_po = test_payment_option_with_rate(
        &invoice.id,
        137,
        "POL",
        None,
        18,
        "200000000000000000000", // 200 POL for $100 (assuming 1 POL = $0.50)
        Some("2".to_string()),   // 1 USD = 2 POL
    );
    PaymentOptionWriter::create(&service, &pol_po)
        .await
        .unwrap();

    // Payment on Ethereum: $50 worth of ETH
    let eth_payment = test_payment_with_credit(
        &invoice.id,
        Some(eth_po.id.0),
        1,
        "ETH",
        "25000000000000000",
        Some("50".to_string()),
        Some("0.0005".to_string()),
    );
    PaymentWriter::upsert(&service, &eth_payment).await.unwrap();

    // Payment on Polygon: $50 worth of POL
    let pol_payment = test_payment_with_credit(
        &invoice.id,
        Some(pol_po.id.0),
        137,
        "POL",
        "100000000000000000000",
        Some("50".to_string()),
        Some("2".to_string()),
    );
    PaymentWriter::upsert(&service, &pol_payment).await.unwrap();

    // Verify total
    let fetched = InvoiceReader::get(&service, &invoice.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fetched.amount_received, "100",
        "Multi-chain payments should aggregate correctly"
    );

    // Verify payments are on different chains
    let payments = PaymentReader::get_for_invoice(&service, &invoice.id)
        .await
        .unwrap();
    let chains: HashSet<_> = payments.iter().map(|p| p.chain_id).collect();
    assert!(chains.contains(&1), "Should have payment on Ethereum");
    assert!(chains.contains(&137), "Should have payment on Polygon");
}

/// E2E test: Fractional credited amounts aggregate correctly
///
/// Scenario:
/// - Create a $100 USD invoice
/// - Three payments with fractional amounts: $33.33 + $33.33 + $33.34 = $100
/// - Verify: amount_received = $100
#[tokio::test]
#[ignore]
async fn integration_fractional_amounts_aggregate() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create a $100 USD invoice
    let mut invoice = test_invoice();
    invoice.currency = "USD".to_string();
    invoice.amount = "100".to_string();
    invoice.amount_received = "0".to_string();
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    // Create payment option
    let eth_po = test_payment_option_with_rate(
        &invoice.id,
        1,
        "ETH",
        None,
        18,
        "50000000000000000",
        Some("0.0005".to_string()),
    );
    PaymentOptionWriter::create(&service, &eth_po)
        .await
        .unwrap();

    // Three payments with fractional amounts
    for (i, amount) in ["33.33", "33.33", "33.34"].iter().enumerate() {
        let payment = test_payment_with_credit(
            &invoice.id,
            Some(eth_po.id.0),
            1,
            "ETH",
            &format!("{}0000000000000", 16665 + i), // Arbitrary wei amounts
            Some(amount.to_string()),
            Some("0.0005".to_string()),
        );
        PaymentWriter::upsert(&service, &payment).await.unwrap();
    }

    // Verify total is exactly $100
    let fetched = InvoiceReader::get(&service, &invoice.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fetched.amount_received, "100.00",
        "Fractional amounts should sum to exactly $100"
    );
}
