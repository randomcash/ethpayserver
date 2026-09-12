#![allow(clippy::unwrap_used, clippy::expect_used)]

//! What a refund is allowed to be worth.
//!
//! The refund amount arrived from the client as a string and was written into
//! the refund record untouched — never parsed, never compared to the payment it
//! refunds, and never compared to the refunds already recorded against that
//! payment. These tests pin both halves of the rule that replaced it: a refund
//! may not exceed its payment, and the payment's value can only be handed back
//! once.

use super::{already_refunded, refundable_amount, resolve_refund_amount};

use alloy_primitives::U256;
use async_trait::async_trait;
use axum::http::StatusCode;
use chrono::Utc;
use types::{
    AssetType, ChainId, InvoiceId, PaymentData, RefundData, RefundReader, RefundStatus,
    RepositoryResult, StoreId,
};
use uuid::Uuid;

fn payment(amount: &str) -> PaymentData {
    PaymentData {
        id: Uuid::new_v4(),
        invoice_id: InvoiceId::from_string("inv-1".to_string()),
        payment_option_id: None,
        chain_id: ChainId::evm(1),
        asset_type: AssetType::Native,
        amount: amount.to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        tx_hash: format!("0x{:064x}", 1),
        block_number: Some(1),
        detected_at: Utc::now(),
        confirmed_at: Some(Utc::now()),
        from_address: Some("0xpayer".to_string()),
        reorged: false,
        extra: None,
        credited_amount: None,
        rate_used: None,
        rate_applied_at: None,
    }
}

fn refund_of(payment_id: Uuid, amount: &str, status: RefundStatus) -> RefundData {
    RefundData {
        id: Uuid::new_v4(),
        invoice_id: InvoiceId::from_string("inv-1".to_string()),
        payment_id,
        store_id: StoreId(Uuid::new_v4()),
        to_address: "0xpayer".to_string(),
        chain_id: ChainId::evm(1),
        asset_type: "native".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        amount: amount.to_string(),
        tx_hash: None,
        status,
        fee_amount: None,
        reason: None,
        error_message: None,
        created_at: Utc::now(),
        confirmed_at: None,
    }
}

/// Returns a fixed set of refunds for any invoice.
struct StubRefunds(Vec<RefundData>);

#[async_trait]
impl RefundReader for StubRefunds {
    async fn get_refunds_for_invoice(&self, _: &InvoiceId) -> RepositoryResult<Vec<RefundData>> {
        Ok(self.0.clone())
    }

    async fn get_refund(&self, _: Uuid) -> RepositoryResult<Option<RefundData>> {
        unimplemented!("not exercised by the refund amount rule")
    }
    async fn get_refunds_for_store(
        &self,
        _: StoreId,
        _: i64,
        _: i64,
    ) -> RepositoryResult<(i64, Vec<RefundData>)> {
        unimplemented!("not exercised by the refund amount rule")
    }
    async fn get_active_refunds(&self) -> RepositoryResult<Vec<RefundData>> {
        unimplemented!("not exercised by the refund amount rule")
    }
}

// =============================================================================
// The amount itself
// =============================================================================

#[test]
fn an_omitted_amount_is_the_whole_payment() {
    let amount = resolve_refund_amount(None, U256::from(1000), U256::ZERO).unwrap();
    assert_eq!(amount, U256::from(1000));
}

#[test]
fn a_partial_amount_within_the_payment_is_allowed() {
    let amount = resolve_refund_amount(Some("400"), U256::from(1000), U256::ZERO).unwrap();
    assert_eq!(amount, U256::from(400));
}

#[test]
fn an_amount_larger_than_the_payment_is_refused() {
    assert_eq!(
        resolve_refund_amount(Some("1001"), U256::from(1000), U256::ZERO),
        Err(StatusCode::BAD_REQUEST),
        "a refund may never be worth more than the payment it refunds"
    );
}

#[test]
fn an_amount_that_is_not_a_base_ten_integer_is_refused() {
    for bad in [
        "", " ", "abc", "-1", "1.5", "1e9", "1_000", "0x10", " 100", "100 ",
    ] {
        assert_eq!(
            resolve_refund_amount(Some(bad), U256::from(1000), U256::ZERO),
            Err(StatusCode::BAD_REQUEST),
            "{bad:?} is not an amount in base units"
        );
    }
}

#[test]
fn a_zero_amount_is_refused() {
    assert_eq!(
        resolve_refund_amount(Some("0"), U256::from(1000), U256::ZERO),
        Err(StatusCode::BAD_REQUEST)
    );
}

// =============================================================================
// Paying the same money back twice
// =============================================================================

#[test]
fn a_fully_refunded_payment_has_nothing_left() {
    assert_eq!(
        resolve_refund_amount(None, U256::from(1000), U256::from(1000)),
        Err(StatusCode::CONFLICT),
        "a payment refunded in full must not be refunded again"
    );
}

#[test]
fn a_partly_refunded_payment_gives_up_only_the_remainder() {
    assert_eq!(
        resolve_refund_amount(Some("600"), U256::from(1000), U256::from(400)),
        Ok(U256::from(600))
    );
    assert_eq!(
        resolve_refund_amount(Some("601"), U256::from(1000), U256::from(400)),
        Err(StatusCode::CONFLICT),
        "the remainder is what is left after existing refunds, not the payment"
    );
}

#[test]
fn refunds_in_flight_hold_their_value() {
    let id = Uuid::new_v4();
    for status in [
        RefundStatus::Pending,
        RefundStatus::Broadcasting,
        RefundStatus::Confirmed,
    ] {
        let refunds = vec![refund_of(id, "1000", status)];
        assert_eq!(
            already_refunded(&refunds, id),
            Some(U256::from(1000)),
            "a {status:?} refund has not failed, so its value is spoken for"
        );
    }
}

#[test]
fn a_failed_refund_releases_its_value() {
    let id = Uuid::new_v4();
    let refunds = vec![refund_of(id, "1000", RefundStatus::Failed)];
    assert_eq!(already_refunded(&refunds, id), Some(U256::ZERO));
}

#[test]
fn refunds_of_another_payment_do_not_count() {
    // One invoice can carry several payments; each is refundable on its own.
    let mine = Uuid::new_v4();
    let other = Uuid::new_v4();
    let refunds = vec![refund_of(other, "1000", RefundStatus::Confirmed)];
    assert_eq!(already_refunded(&refunds, mine), Some(U256::ZERO));
}

#[test]
fn an_unparseable_stored_refund_is_not_silently_skipped() {
    // Skipping it would undercount what has already gone out and let a refund
    // be written on top of one that is already there.
    let id = Uuid::new_v4();
    let refunds = vec![refund_of(id, "not-a-number", RefundStatus::Confirmed)];
    assert_eq!(already_refunded(&refunds, id), None);
}

// =============================================================================
// The two joined together, over a refund store
// =============================================================================

#[tokio::test]
async fn a_first_full_refund_is_allowed() {
    let payment = payment("1000");
    let store = StubRefunds(Vec::new());

    let amount = refundable_amount(&store, &payment.invoice_id, &payment, None)
        .await
        .expect("an unrefunded payment may be refunded in full");

    assert_eq!(amount, U256::from(1000));
}

#[tokio::test]
async fn a_second_full_refund_is_refused() {
    let payment = payment("1000");
    let store = StubRefunds(vec![refund_of(payment.id, "1000", RefundStatus::Confirmed)]);

    let refused = refundable_amount(&store, &payment.invoice_id, &payment, None).await;

    assert_eq!(
        refused,
        Err(StatusCode::CONFLICT),
        "the refunds already recorded against a payment bound the next one"
    );
}

#[tokio::test]
async fn an_over_large_amount_is_refused_against_a_real_payment() {
    let payment = payment("1000");
    let store = StubRefunds(Vec::new());

    let refused = refundable_amount(&store, &payment.invoice_id, &payment, Some("100000")).await;

    assert_eq!(refused, Err(StatusCode::BAD_REQUEST));
}
