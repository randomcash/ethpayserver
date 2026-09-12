//! Payout/settlement API endpoints.
//!
//! POST /stores/{store_id}/payouts — Initiate a payout (sweep funds to merchant wallet).
//! GET  /stores/{store_id}/payouts — List payouts for a store.
//! GET  /stores/{store_id}/payouts/{payout_id} — Get payout details.
//!
//! Two rules hold everything here together, and both are enforced below rather
//! than assumed: a payout may only be computed from invoices the store in the
//! path owns, and an invoice's money may only be claimed once.

#[cfg(test)]
mod tests;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::Utc;
use uuid::Uuid;

use alloy_primitives::U256;
use auth::{SessionService, UserStoreRepository};
use data_service::{InvoiceReader, PaymentReader, PayoutClaimReader, PayoutReader, PayoutWriter};
use types::{PayoutData, PayoutStatus, StoreId};

use super::extractors::AuthenticatedUser;
use crate::metrics;
use crate::state::PgAppState;
pub use api_types::{CreatePayoutRequest, PayoutListResponse, PayoutResponse};

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

/// Sum what `invoice_ids` are worth, refusing any invoice the store does not own.
///
/// The caller supplies the invoice ids and the destination address; membership
/// of the store in the path is all that has been checked by this point. So each
/// invoice is loaded and matched against that store — an id the store does not
/// own contributes nothing and stops the request.
///
/// The refusal is 404, the same answer as an invoice that does not exist. A 403
/// would separate "not yours" from "no such invoice", and that difference is
/// itself another merchant's data.
async fn payable_total<R>(
    reader: &R,
    store_id: StoreId,
    invoice_ids: &[String],
    chain_id: &types::ChainId,
    asset_symbol: &str,
) -> Result<U256, StatusCode>
where
    R: InvoiceReader + PaymentReader,
{
    let mut total = U256::ZERO;

    for invoice_id_str in invoice_ids {
        let invoice_id = types::InvoiceId::from_string(invoice_id_str.clone());

        let invoice = InvoiceReader::get(reader, &invoice_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;

        if invoice.store_id != store_id {
            tracing::warn!(
                store_id = %store_id,
                "Payout named an invoice that belongs to another store"
            );
            return Err(StatusCode::NOT_FOUND);
        }

        let payments = PaymentReader::get_valid_for_invoice(reader, &invoice_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        total += sum_payable(&payments, chain_id, asset_symbol);
    }

    Ok(total)
}

/// Refuse a payout over invoices an existing payout already claims.
///
/// Without this the same invoices can be paid out again and again, each payout
/// recomputing the full total from the same payments. Answered 409: the request
/// is well formed, the state is what refuses it.
async fn reject_claimed_invoices<R>(
    reader: &R,
    store_id: StoreId,
    invoice_ids: &[String],
) -> Result<(), StatusCode>
where
    R: PayoutClaimReader,
{
    let claimed = reader
        .invoice_ids_already_claimed(store_id, invoice_ids)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to read existing payout claims");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if claimed.is_empty() {
        return Ok(());
    }

    tracing::warn!(
        store_id = %store_id,
        claimed = claimed.len(),
        "Payout refused: these invoices are already claimed by another payout"
    );
    Err(StatusCode::CONFLICT)
}

/// Load a payout, refusing one that belongs to a different store.
///
/// The path carries both ids and only the store id is checked by the membership
/// gate, so the payout itself must be matched to it. Mismatch is 404 for the
/// same reason as in [`payable_total`].
async fn payout_for_store<R>(
    reader: &R,
    store_id: StoreId,
    payout_id: Uuid,
) -> Result<PayoutData, StatusCode>
where
    R: PayoutReader,
{
    let payout = PayoutReader::get_payout(reader, payout_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if payout.store_id != store_id {
        tracing::warn!(
            store_id = %store_id,
            "Payout lookup refused: the payout belongs to another store"
        );
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(payout)
}

/// Initiate a payout — record an intent to sweep funds to a merchant wallet.
///
/// This writes a payout row and nothing else. No transaction is signed or
/// broadcast anywhere in this deployment: the server holds only public xpubs, so
/// the record stays `Pending` until something outside this process acts on it.
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

    // Ownership first, then whether the money is still unclaimed: an invoice
    // this store does not own is a 404 whatever its payout history says.
    let total_amount = payable_total(
        &*state.data_service,
        store_id,
        &body.invoice_ids,
        &body.chain_id,
        &body.asset_symbol,
    )
    .await?;

    reject_claimed_invoices(&*state.data_service, store_id, &body.invoice_ids).await?;

    if total_amount.is_zero() {
        tracing::warn!(store_id = %store_id, "No confirmed payments found for payout");
        return Err(StatusCode::BAD_REQUEST);
    }

    let asset_type = if body.token_address.is_some() {
        "erc20"
    } else {
        "native"
    }
    .to_string();

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

    let payout = payout_for_store(&*state.data_service, store_id, payout_id).await?;

    Ok(Json(payout.into()))
}
