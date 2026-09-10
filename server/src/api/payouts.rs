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
use uuid::Uuid;

use alloy_primitives::U256;
use auth::{SessionService, UserStoreRepository};
use data_service::{PaymentReader, PayoutReader, PayoutWriter};
use types::{PayoutData, PayoutStatus, StoreId};

use super::extractors::AuthenticatedUser;
use crate::metrics;
use crate::state::PgAppState;
pub use api_types::{CreatePayoutRequest, PayoutListResponse, PayoutResponse};

/// Initiate a payout — sweep funds from derived addresses to merchant wallet.
///
/// Creates a payout record. The actual transaction signing and broadcasting
/// is handled by a background service that monitors pending payouts.
/// Total of the payments on this chain and asset that may be paid out.
///
/// Reorged and unconfirmed payments are excluded; an unparseable amount is
/// skipped rather than failing the payout, matching the behaviour before
/// chain ids became CAIP-2.
fn sum_payable(
    payments: &[types::PaymentData],
    chain_id: &types::ChainId,
    asset_symbol: &str,
) -> U256 {
    payments
        .iter()
        .filter(|p| {
            p.confirmed_at.is_some()
                && !p.reorged
                && &p.chain_id == chain_id
                && p.asset_symbol == asset_symbol
        })
        .filter_map(|p| p.amount.parse::<U256>().ok())
        .fold(U256::ZERO, |acc, amt| acc + amt)
}

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

    // Validate destination address is not empty
    if body.destination_address.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Validate invoice_ids are not empty
    if body.invoice_ids.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let asset_type = if body.token_address.is_some() {
        "erc20"
    } else {
        "native"
    }
    .to_string();

    // Calculate total amount from confirmed payments for the specified invoices
    let mut total_amount = U256::ZERO;
    for invoice_id_str in &body.invoice_ids {
        let invoice_id = types::InvoiceId::from_string(invoice_id_str.clone());
        let payments = PaymentReader::get_valid_for_invoice(&*state.data_service, &invoice_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        total_amount += sum_payable(&payments, &body.chain_id, &body.asset_symbol);
    }

    if total_amount.is_zero() {
        tracing::warn!(store_id = %store_id, "No confirmed payments found for payout");
        return Err(StatusCode::BAD_REQUEST);
    }

    let payout = PayoutData {
        id: Uuid::new_v4(),
        store_id,
        invoice_ids: body.invoice_ids,
        destination_address: body.destination_address,
        chain_id: body.chain_id.clone(),
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

    metrics::record_payout_initiated(&body.chain_id, &body.asset_symbol);

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
