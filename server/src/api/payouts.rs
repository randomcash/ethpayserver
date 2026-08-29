//! Payout/settlement API endpoints.
//!
//! POST /stores/{store_id}/payouts — Initiate a payout (sweep funds to merchant wallet).
//! GET  /stores/{store_id}/payouts — List payouts for a store.
//! GET  /stores/{store_id}/payouts/{payout_id} — Get payout details.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use alloy_primitives::U256;
use auth::{SessionService, UserStoreRepository};
use data_service::{PaymentReader, PayoutReader, PayoutWriter};
use types::{PaymentData, PayoutData, PayoutStatus, StoreId};

use super::extractors::AuthenticatedUser;
use crate::metrics;
use crate::state::PgAppState;

/// Request body for creating a payout.
#[derive(Debug, Deserialize)]
pub struct CreatePayoutRequest {
    /// Invoice IDs to include in this payout.
    /// If empty, sweeps all settled invoices.
    pub invoice_ids: Vec<String>,
    /// Destination wallet address.
    pub destination_address: String,
    /// EIP-155 chain ID to sweep from.
    pub chain_id: u64,
    /// Asset symbol to sweep (e.g., "ETH", "USDC").
    pub asset_symbol: String,
    /// Token contract address (required for ERC20 payouts).
    pub token_address: Option<String>,
}

/// Payout response.
#[derive(Debug, Serialize)]
pub struct PayoutResponse {
    pub id: Uuid,
    pub store_id: Uuid,
    pub invoice_ids: Vec<String>,
    pub destination_address: String,
    pub chain_id: u64,
    pub asset_type: String,
    pub asset_symbol: String,
    pub amount: String,
    pub tx_hash: Option<String>,
    pub status: String,
    pub fee_amount: Option<String>,
    pub error_message: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
    pub confirmed_at: Option<chrono::DateTime<Utc>>,
}

impl From<PayoutData> for PayoutResponse {
    fn from(p: PayoutData) -> Self {
        Self {
            id: p.id,
            store_id: p.store_id.0,
            invoice_ids: p.invoice_ids,
            destination_address: p.destination_address,
            chain_id: p.chain_id,
            asset_type: p.asset_type,
            asset_symbol: p.asset_symbol,
            amount: p.amount,
            tx_hash: p.tx_hash,
            status: p.status.to_string(),
            fee_amount: p.fee_amount,
            error_message: p.error_message,
            created_at: p.created_at,
            confirmed_at: p.confirmed_at,
        }
    }
}

/// Payout list response with pagination.
#[derive(Debug, Serialize)]
pub struct PayoutListResponse {
    pub total: i64,
    pub payouts: Vec<PayoutResponse>,
}

// ============================================================================
// Payout decision logic
//
// Extracted from `create_payout` so the validation and amount-aggregation
// rules can be unit tested without a live `PgAppState` (which is bound to a
// real Postgres pool). The handler stays the only place that emits logs /
// touches the DB.
// ============================================================================

/// Validate the parts of a payout request that need no database access.
///
/// A payout with no destination has nowhere to send funds, and one with no
/// invoices has nothing to sweep — both are client errors.
pub(crate) fn validate_payout_request(body: &CreatePayoutRequest) -> Result<(), StatusCode> {
    if body.destination_address.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    if body.invoice_ids.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    Ok(())
}

/// Asset type recorded for a payout: an ERC20 payout carries a token contract,
/// a native one does not.
pub(crate) fn payout_asset_type(token_address: Option<&str>) -> &'static str {
    if token_address.is_some() {
        "erc20"
    } else {
        "native"
    }
}

/// Total the payments that this payout is allowed to sweep.
///
/// Counts only confirmed, non-reorged payments on the requested chain and
/// asset. An unparseable amount is skipped rather than failing the payout,
/// matching the handler's original `if let Ok(..)` behaviour.
pub(crate) fn sum_sweepable_payments(
    payments: &[PaymentData],
    chain_id: u64,
    asset_symbol: &str,
) -> U256 {
    let mut total = U256::ZERO;
    for payment in payments {
        if payment.confirmed_at.is_some()
            && !payment.reorged
            && payment.chain_id == chain_id
            && payment.asset_symbol == asset_symbol
            && let Ok(amt) = payment.amount.parse::<U256>()
        {
            total += amt;
        }
    }
    total
}

/// Initiate a payout — sweep funds from derived addresses to merchant wallet.
///
/// Creates a payout record. The actual transaction signing and broadcasting
/// is handled by a background service that monitors pending payouts.
pub async fn create_payout<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
    Json(body): Json<CreatePayoutRequest>,
) -> Result<Json<PayoutResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let store_id = StoreId(store_id);

    // Verify user has access to this store
    if !user.role.is_admin()
        && state
            .data_service
            .get_user_store(user.id, store_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }

    validate_payout_request(&body)?;

    let asset_type = payout_asset_type(body.token_address.as_deref()).to_string();

    // Calculate total amount from confirmed payments for the specified invoices
    let mut total_amount = U256::ZERO;
    for invoice_id_str in &body.invoice_ids {
        let invoice_id = types::InvoiceId::from_string(invoice_id_str.clone());
        let payments = PaymentReader::get_valid_for_invoice(&*state.data_service, &invoice_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        total_amount += sum_sweepable_payments(&payments, body.chain_id, &body.asset_symbol);
    }

    if total_amount.is_zero() {
        tracing::warn!(
            store_id = %store_id,
            "No confirmed payments found for payout"
        );
        return Err(StatusCode::BAD_REQUEST);
    }

    let payout = PayoutData {
        id: Uuid::new_v4(),
        store_id,
        invoice_ids: body.invoice_ids,
        destination_address: body.destination_address,
        chain_id: body.chain_id,
        asset_type,
        asset_symbol: body.asset_symbol.clone(),
        token_address: body.token_address,
        amount: total_amount.to_string(),
        tx_hash: None,
        status: PayoutStatus::Pending,
        fee_amount: None,
        error_message: None,
        created_at: Utc::now(),
        confirmed_at: None,
    };

    PayoutWriter::create_payout(&*state.data_service, &payout)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to create payout record");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    metrics::record_payout_initiated(body.chain_id, &body.asset_symbol);

    tracing::info!(
        payout_id = %payout.id,
        store_id = %store_id,
        amount = %payout.amount,
        destination = %payout.destination_address,
        "Payout created"
    );

    Ok(Json(payout.into()))
}

/// List payouts for a store.
pub async fn list_payouts<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
) -> Result<Json<PayoutListResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let store_id = StoreId(store_id);

    // Verify user has access to this store
    if !user.role.is_admin()
        && state
            .data_service
            .get_user_store(user.id, store_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }

    let (total, payouts) =
        PayoutReader::get_payouts_for_store(&*state.data_service, store_id, 50, 0)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(PayoutListResponse {
        total,
        payouts: payouts.into_iter().map(Into::into).collect::<Vec<_>>(),
    }))
}

/// Get a specific payout.
pub async fn get_payout<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path((store_id, payout_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<PayoutResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    // Verify user has access to this store
    if !user.role.is_admin()
        && state
            .data_service
            .get_user_store(user.id, StoreId(store_id))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }

    let payout = PayoutReader::get_payout(&*state.data_service, payout_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(payout.into()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    use types::{AssetType, InvoiceId};

    const TEST_CHAIN_ID: u64 = 11155111;
    const MERCHANT_WALLET: &str = "0x1111111111111111111111111111111111111111";
    const USDC: &str = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48";

    fn payout_request(invoice_ids: Vec<String>) -> CreatePayoutRequest {
        CreatePayoutRequest {
            invoice_ids,
            destination_address: MERCHANT_WALLET.to_string(),
            chain_id: TEST_CHAIN_ID,
            asset_symbol: "ETH".to_string(),
            token_address: None,
        }
    }

    /// A confirmed, non-reorged ETH payment on the sweep chain — the
    /// sweepable baseline that individual tests mutate one field at a time.
    fn sweepable_payment(invoice_id: &InvoiceId, amount: &str) -> PaymentData {
        PaymentData {
            id: Uuid::new_v4(),
            invoice_id: invoice_id.clone(),
            payment_option_id: None,
            chain_id: TEST_CHAIN_ID,
            asset_type: AssetType::Native,
            amount: amount.to_string(),
            asset_symbol: "ETH".to_string(),
            token_address: None,
            tx_hash: format!("0x{:064x}", Uuid::new_v4().as_u128()),
            block_number: Some(12_345_678),
            detected_at: Utc::now(),
            confirmed_at: Some(Utc::now()),
            from_address: Some("0xabcdef1234567890abcdef1234567890abcdef12".to_string()),
            reorged: false,
            extra: None,
            credited_amount: None,
            rate_used: None,
            rate_applied_at: None,
        }
    }

    fn test_payout(status: PayoutStatus) -> PayoutData {
        PayoutData {
            id: Uuid::new_v4(),
            store_id: StoreId::new(),
            invoice_ids: vec!["inv_1".to_string(), "inv_2".to_string()],
            destination_address: MERCHANT_WALLET.to_string(),
            chain_id: TEST_CHAIN_ID,
            asset_type: "native".to_string(),
            asset_symbol: "ETH".to_string(),
            token_address: None,
            amount: "3000000000000000000".to_string(),
            tx_hash: None,
            status,
            fee_amount: None,
            error_message: None,
            created_at: Utc::now(),
            confirmed_at: None,
        }
    }

    // ------------------------------------------------------------------
    // Payout validation
    // ------------------------------------------------------------------

    #[test]
    fn accepts_a_well_formed_payout_request() {
        let body = payout_request(vec!["inv_1".to_string()]);
        assert!(validate_payout_request(&body).is_ok());
    }

    /// Without a destination the sweep has nowhere to send funds.
    #[test]
    fn rejects_an_empty_destination_address() {
        let mut body = payout_request(vec!["inv_1".to_string()]);
        body.destination_address = String::new();

        assert_eq!(
            validate_payout_request(&body).unwrap_err(),
            StatusCode::BAD_REQUEST
        );
    }

    /// A whitespace-only destination is just as unusable as an empty one, and
    /// would otherwise be stored verbatim as the payout target.
    #[test]
    fn rejects_a_whitespace_only_destination_address() {
        let mut body = payout_request(vec!["inv_1".to_string()]);
        body.destination_address = "   \t ".to_string();

        assert_eq!(
            validate_payout_request(&body).unwrap_err(),
            StatusCode::BAD_REQUEST
        );
    }

    /// The doc comment on `invoice_ids` advertises "sweep everything" on an
    /// empty list, but the handler rejects it. Pin the implemented behaviour.
    #[test]
    fn rejects_an_empty_invoice_id_list() {
        let body = payout_request(vec![]);

        assert_eq!(
            validate_payout_request(&body).unwrap_err(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn asset_type_follows_the_token_address() {
        assert_eq!(payout_asset_type(None), "native");
        assert_eq!(payout_asset_type(Some(USDC)), "erc20");
    }

    // ------------------------------------------------------------------
    // Payout amount aggregation
    // ------------------------------------------------------------------

    #[test]
    fn sums_confirmed_payments_on_the_requested_chain_and_asset() {
        let invoice_id = InvoiceId::new();
        let payments = vec![
            sweepable_payment(&invoice_id, "1000000000000000000"),
            sweepable_payment(&invoice_id, "2000000000000000000"),
        ];

        let total = sum_sweepable_payments(&payments, TEST_CHAIN_ID, "ETH");
        assert_eq!(total.to_string(), "3000000000000000000");
    }

    #[test]
    fn no_payments_sums_to_zero() {
        let total = sum_sweepable_payments(&[], TEST_CHAIN_ID, "ETH");
        assert!(total.is_zero());
    }

    /// Sweeping unconfirmed funds would pay out money the merchant may never
    /// receive if the transaction never lands.
    #[test]
    fn excludes_unconfirmed_payments() {
        let invoice_id = InvoiceId::new();
        let mut payment = sweepable_payment(&invoice_id, "1000000000000000000");
        payment.confirmed_at = None;

        assert!(sum_sweepable_payments(&[payment], TEST_CHAIN_ID, "ETH").is_zero());
    }

    /// A reorged payment was rolled back on-chain; the funds are not there.
    #[test]
    fn excludes_reorged_payments() {
        let invoice_id = InvoiceId::new();
        let mut payment = sweepable_payment(&invoice_id, "1000000000000000000");
        payment.reorged = true;

        assert!(sum_sweepable_payments(&[payment], TEST_CHAIN_ID, "ETH").is_zero());
    }

    /// Funds on a different chain cannot be swept by this payout — mixing them
    /// into the total would create a payout for more than the chain holds.
    #[test]
    fn excludes_payments_on_another_chain() {
        let invoice_id = InvoiceId::new();
        let mut other_chain = sweepable_payment(&invoice_id, "5000000000000000000");
        other_chain.chain_id = 1;

        let payments = vec![
            sweepable_payment(&invoice_id, "1000000000000000000"),
            other_chain,
        ];

        let total = sum_sweepable_payments(&payments, TEST_CHAIN_ID, "ETH");
        assert_eq!(total.to_string(), "1000000000000000000");
    }

    /// Smallest units are asset-specific: adding USDC (6 decimals) into an ETH
    /// (18 decimals) sweep total would be meaningless arithmetic.
    #[test]
    fn excludes_payments_in_another_asset() {
        let invoice_id = InvoiceId::new();
        let mut usdc = sweepable_payment(&invoice_id, "5000000");
        usdc.asset_symbol = "USDC".to_string();
        usdc.asset_type = AssetType::ERC20;
        usdc.token_address = Some(USDC.to_string());

        let payments = vec![sweepable_payment(&invoice_id, "1000000000000000000"), usdc];

        let total = sum_sweepable_payments(&payments, TEST_CHAIN_ID, "ETH");
        assert_eq!(total.to_string(), "1000000000000000000");
    }

    /// An amount the database cannot parse as a U256 is skipped rather than
    /// aborting the sweep — pins the handler's original `if let Ok(..)` branch.
    #[test]
    fn skips_unparseable_amounts_without_failing_the_sweep() {
        let invoice_id = InvoiceId::new();
        let mut broken = sweepable_payment(&invoice_id, "not-a-number");
        broken.id = Uuid::new_v4();

        let payments = vec![broken, sweepable_payment(&invoice_id, "1000000000000000000")];

        let total = sum_sweepable_payments(&payments, TEST_CHAIN_ID, "ETH");
        assert_eq!(total.to_string(), "1000000000000000000");
    }

    /// A payout whose sweepable total is zero is rejected by the handler; this
    /// pins the condition that decision reads.
    #[test]
    fn only_ineligible_payments_sum_to_zero() {
        let invoice_id = InvoiceId::new();
        let mut unconfirmed = sweepable_payment(&invoice_id, "1000000000000000000");
        unconfirmed.confirmed_at = None;
        let mut reorged = sweepable_payment(&invoice_id, "2000000000000000000");
        reorged.reorged = true;

        let total = sum_sweepable_payments(&[unconfirmed, reorged], TEST_CHAIN_ID, "ETH");
        assert!(total.is_zero());
    }

    // ------------------------------------------------------------------
    // Payout status transitions, as seen through the API response
    // ------------------------------------------------------------------

    /// The wire format must stay snake_case: merchant integrations match on
    /// these exact strings.
    #[test]
    fn response_serialises_each_payout_status() {
        for (status, expected) in [
            (PayoutStatus::Pending, "pending"),
            (PayoutStatus::Broadcasting, "broadcasting"),
            (PayoutStatus::Confirmed, "confirmed"),
            (PayoutStatus::Failed, "failed"),
        ] {
            let response = PayoutResponse::from(test_payout(status));
            assert_eq!(response.status, expected);
        }
    }

    #[test]
    fn pending_and_broadcasting_are_not_final_but_confirmed_and_failed_are() {
        assert!(!PayoutStatus::Pending.is_final());
        assert!(!PayoutStatus::Broadcasting.is_final());
        assert!(PayoutStatus::Confirmed.is_final());
        assert!(PayoutStatus::Failed.is_final());
    }

    /// The lifecycle a payout walks: created pending, broadcast, then settled.
    #[test]
    fn payout_lifecycle_reaches_a_final_status() {
        let lifecycle = [
            PayoutStatus::Pending,
            PayoutStatus::Broadcasting,
            PayoutStatus::Confirmed,
        ];

        let (last, leading) = lifecycle.split_last().unwrap();
        for status in leading {
            assert!(!status.is_final(), "{status} is mid-flight");
        }
        assert!(last.is_final(), "the lifecycle ends in a final status");
    }

    #[test]
    fn payout_status_round_trips_through_its_wire_string() {
        for status in [
            PayoutStatus::Pending,
            PayoutStatus::Broadcasting,
            PayoutStatus::Confirmed,
            PayoutStatus::Failed,
        ] {
            let parsed: PayoutStatus = status.as_str().parse().unwrap();
            assert_eq!(parsed, status);
        }
    }

    // ------------------------------------------------------------------
    // Response mapping
    // ------------------------------------------------------------------

    /// A freshly created payout is pending with no transaction yet — the
    /// broadcasting service fills `tx_hash`/`fee_amount`/`confirmed_at` later.
    #[test]
    fn newly_created_payout_maps_to_a_pending_response() {
        let payout = test_payout(PayoutStatus::Pending);
        let expected_store = payout.store_id.0;

        let response = PayoutResponse::from(payout.clone());

        assert_eq!(response.id, payout.id);
        assert_eq!(response.store_id, expected_store);
        assert_eq!(response.invoice_ids, vec!["inv_1", "inv_2"]);
        assert_eq!(response.destination_address, MERCHANT_WALLET);
        assert_eq!(response.chain_id, TEST_CHAIN_ID);
        assert_eq!(response.asset_type, "native");
        assert_eq!(response.amount, "3000000000000000000");
        assert_eq!(response.status, "pending");
        assert!(response.tx_hash.is_none());
        assert!(response.confirmed_at.is_none());
    }

    #[test]
    fn failed_payout_response_carries_the_error_message() {
        let mut payout = test_payout(PayoutStatus::Failed);
        payout.error_message = Some("gas estimation failed".to_string());

        let response = PayoutResponse::from(payout);

        assert_eq!(response.status, "failed");
        assert_eq!(
            response.error_message.as_deref(),
            Some("gas estimation failed")
        );
    }

    #[test]
    fn list_response_reports_the_total_alongside_the_page() {
        let listed = PayoutListResponse {
            total: 2,
            payouts: vec![
                PayoutResponse::from(test_payout(PayoutStatus::Confirmed)),
                PayoutResponse::from(test_payout(PayoutStatus::Pending)),
            ],
        };

        assert_eq!(listed.total, 2);
        assert_eq!(listed.payouts.len(), 2);
        assert_eq!(listed.payouts[0].status, "confirmed");
        assert_eq!(listed.payouts[1].status, "pending");
    }
}
