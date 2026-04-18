//! Payout/settlement API endpoints.
//!
//! POST /stores/{store_id}/payouts — Initiate a payout (sweep funds to merchant wallet).
//! GET  /stores/{store_id}/payouts — List payouts for a store.
//! GET  /stores/{store_id}/payouts/{payout_id} — Get payout details.

use axum::{
    Json,
    extract::{Path, State},
};
use chrono::Utc;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use auth::SessionService;
use data_service::{PaymentReader, PayoutReader, PayoutWriter, StoreWalletReader};
use types::{PayoutData, PayoutStatus, StoreId};

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

/// Initiate a payout — sweep funds from derived addresses to merchant wallet.
///
/// Creates a payout record. The actual transaction signing and broadcasting
/// is handled by a background service that monitors pending payouts.
pub async fn create_payout<A>(
    AuthenticatedUser(_user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
    Json(body): Json<CreatePayoutRequest>,
) -> Result<Json<PayoutResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let store_id = StoreId(store_id);

    // Validate destination address is not empty
    if body.destination_address.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Validate invoice_ids are not empty
    if body.invoice_ids.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Determine asset type from token_address
    let asset_type = if body.token_address.is_some() {
        "erc20".to_string()
    } else {
        "native".to_string()
    };

    // Calculate total amount from confirmed payments for the specified invoices
    let mut total_amount = alloy::primitives::U256::ZERO;
    for invoice_id_str in &body.invoice_ids {
        let invoice_id = types::InvoiceId::from_string(invoice_id_str.clone());
        let payments = PaymentReader::get_valid_for_invoice(&*state.data_service, &invoice_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        for payment in payments {
            if payment.confirmed_at.is_some()
                && !payment.reorged
                && payment.chain_id == body.chain_id
                && payment.asset_symbol == body.asset_symbol
            {
                if let Ok(amt) = payment.amount.parse::<alloy::primitives::U256>() {
                    total_amount += amt;
                }
            }
        }
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
    AuthenticatedUser(_user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
) -> Result<Json<PayoutListResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let store_id = StoreId(store_id);

    let (total, payouts) =
        PayoutReader::get_payouts_for_store(&*state.data_service, store_id, 50, 0)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(PayoutListResponse {
        total,
        payouts: payouts.into_iter().map(Into::into).collect(),
    }))
}

/// Get a specific payout.
pub async fn get_payout<A>(
    AuthenticatedUser(_user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path((_store_id, payout_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<PayoutResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let payout = PayoutReader::get_payout(&*state.data_service, payout_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(payout.into()))
}
