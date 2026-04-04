//! Integration tests for PostgreSQL repository implementations.
//!
//! These tests require a running PostgreSQL instance with the schema applied.
//! Run with: `DATABASE_URL="postgres://..." cargo test -p data-service -- --ignored`
//!
//! Test modules:
//! - `invoice`: Invoice CRUD and query tests
//! - `payment`: Payment CRUD and confirmation tests
//! - `watched_address`: Watched address management tests
//! - `aggregation`: Multi-currency payment aggregation E2E tests

mod aggregation;
mod invoice;
mod payment;
mod watched_address;

use chrono::Utc;
use types::{PaymentMethodId, PaymentOptionData, PaymentOptionId};

use super::tests::{create_test_service, test_invoice, test_payment, unique_address};

// =============================================================================
// Shared test helpers
// =============================================================================

/// Create a basic payment option for testing.
pub(crate) fn test_payment_option(
    invoice_id: &types::InvoiceId,
    chain_id: u64,
) -> PaymentOptionData {
    PaymentOptionData {
        id: PaymentOptionId::new(),
        invoice_id: invoice_id.clone(),
        payment_method_id: PaymentMethodId::new("ETH", chain_id),
        chain_id,
        asset_symbol: "ETH".to_string(),
        token_address: None,
        decimals: 18,
        payment_address: format!("0x{:040x}", uuid::Uuid::new_v4().as_u128()),
        amount: "1000000000000000000".to_string(),
        rate: None,
        rate_at: None,
        is_active: true,
        created_at: Utc::now(),
    }
}

/// Create a payment option with specific asset, rate, and decimals.
pub(crate) fn test_payment_option_with_rate(
    invoice_id: &types::InvoiceId,
    chain_id: u64,
    asset_symbol: &str,
    token_address: Option<String>,
    decimals: u8,
    amount: &str,
    rate: Option<String>,
) -> PaymentOptionData {
    PaymentOptionData {
        id: PaymentOptionId::new(),
        invoice_id: invoice_id.clone(),
        payment_method_id: PaymentMethodId::new(asset_symbol, chain_id),
        chain_id,
        asset_symbol: asset_symbol.to_string(),
        token_address,
        decimals,
        payment_address: format!("0x{:040x}", uuid::Uuid::new_v4().as_u128()),
        amount: amount.to_string(),
        rate,
        rate_at: Some(Utc::now()),
        is_active: true,
        created_at: Utc::now(),
    }
}

/// Create a payment with credited_amount for aggregation testing.
pub(crate) fn test_payment_with_credit(
    invoice_id: &types::InvoiceId,
    payment_option_id: Option<uuid::Uuid>,
    chain_id: u64,
    asset_symbol: &str,
    amount: &str,
    credited_amount: Option<String>,
    rate_used: Option<String>,
) -> types::traits::PaymentData {
    let has_credit = credited_amount.is_some();
    types::traits::PaymentData {
        id: uuid::Uuid::new_v4(),
        invoice_id: invoice_id.clone(),
        payment_option_id,
        chain_id,
        asset_type: types::AssetType::Native,
        amount: amount.to_string(),
        asset_symbol: asset_symbol.to_string(),
        token_address: None,
        tx_hash: format!("0x{:064x}", uuid::Uuid::new_v4().as_u128()),
        block_number: Some(12345678),
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: Some("0xabcdef1234567890abcdef1234567890abcdef12".to_string()),
        reorged: false,
        extra: None,
        credited_amount,
        rate_used,
        rate_applied_at: if has_credit { Some(Utc::now()) } else { None },
    }
}
