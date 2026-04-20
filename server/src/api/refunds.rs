//! Refund API endpoints.
//!
//! POST /invoices/{invoice_id}/refund — Initiate a refund for a paid invoice.
//! GET  /invoices/{invoice_id}/refunds — List refunds for an invoice.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use auth::SessionService;
use data_service::{InvoiceReader, PaymentReader, RefundReader, RefundWriter};
use types::{InvoiceId, InvoiceStatus, RefundData, RefundStatus};

use super::extractors::AuthenticatedUser;
use crate::metrics;
use crate::state::PgAppState;

/// Request body for creating a refund.
#[derive(Debug, Deserialize)]
pub struct CreateRefundRequest {
    /// Optional partial refund amount (in smallest unit).
    /// If omitted, refunds the full payment amount.
    pub amount: Option<String>,
    /// Reason for the refund.
    pub reason: Option<String>,
}

/// Refund response.
#[derive(Debug, Serialize)]
pub struct RefundResponse {
    pub id: Uuid,
    pub invoice_id: String,
    pub payment_id: Uuid,
    pub to_address: String,
    pub chain_id: u64,
    pub asset_type: String,
    pub asset_symbol: String,
    pub amount: String,
    pub tx_hash: Option<String>,
    pub status: String,
    pub fee_amount: Option<String>,
    pub reason: Option<String>,
    pub error_message: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
    pub confirmed_at: Option<chrono::DateTime<Utc>>,
}

impl From<RefundData> for RefundResponse {
    fn from(r: RefundData) -> Self {
        Self {
            id: r.id,
            invoice_id: r.invoice_id.0,
            payment_id: r.payment_id,
            to_address: r.to_address,
            chain_id: r.chain_id,
            asset_type: r.asset_type,
            asset_symbol: r.asset_symbol,
            amount: r.amount,
            tx_hash: r.tx_hash,
            status: r.status.to_string(),
            fee_amount: r.fee_amount,
            reason: r.reason,
            error_message: r.error_message,
            created_at: r.created_at,
            confirmed_at: r.confirmed_at,
        }
    }
}

/// Initiate a refund for a paid invoice.
///
/// Validates the invoice is in Paid or LatePaid status, finds the confirmed
/// payment, and creates a refund record. The actual transaction signing and
/// broadcasting is handled by a background service.
pub async fn create_refund<A>(
    AuthenticatedUser(_user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(invoice_id): Path<String>,
    Json(body): Json<CreateRefundRequest>,
) -> Result<Json<RefundResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let id = InvoiceId::from_string(invoice_id);

    // Get invoice and validate status
    let invoice = InvoiceReader::get(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    match invoice.status {
        InvoiceStatus::Paid | InvoiceStatus::LatePaid => {}
        _ => {
            tracing::warn!(
                invoice_id = %id.0,
                status = %invoice.status,
                "Cannot refund invoice in this status"
            );
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    // Get confirmed payments for this invoice
    let payments = PaymentReader::get_valid_for_invoice(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let payment = payments
        .into_iter()
        .find(|p| p.confirmed_at.is_some() && !p.reorged)
        .ok_or_else(|| {
            tracing::warn!(invoice_id = %id.0, "No confirmed payment found for refund");
            StatusCode::BAD_REQUEST
        })?;

    // Validate from_address exists (needed as refund destination)
    let to_address = payment.from_address.clone().ok_or_else(|| {
        tracing::warn!(
            invoice_id = %id.0,
            payment_id = %payment.id,
            "Payment has no from_address, cannot determine refund destination"
        );
        StatusCode::BAD_REQUEST
    })?;

    // Determine refund amount
    let refund_amount = body.amount.unwrap_or_else(|| payment.amount.clone());

    // Create refund record
    let refund = RefundData {
        id: Uuid::new_v4(),
        invoice_id: id.clone(),
        payment_id: payment.id,
        store_id: invoice.store_id,
        to_address,
        chain_id: payment.chain_id,
        asset_type: payment.asset_type.to_string(),
        asset_symbol: payment.asset_symbol.clone(),
        token_address: payment.token_address.clone(),
        amount: refund_amount,
        tx_hash: None,
        status: RefundStatus::Pending,
        fee_amount: None,
        reason: body.reason,
        error_message: None,
        created_at: Utc::now(),
        confirmed_at: None,
    };

    RefundWriter::create_refund(&*state.data_service, &refund)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to create refund record");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    metrics::record_refund_initiated(payment.chain_id, &payment.asset_symbol);

    tracing::info!(
        refund_id = %refund.id,
        invoice_id = %id.0,
        amount = %refund.amount,
        to_address = %refund.to_address,
        "Refund created"
    );

    Ok(Json(refund.into()))
}

/// List refunds for an invoice.
pub async fn list_refunds<A>(
    AuthenticatedUser(_user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(invoice_id): Path<String>,
) -> Result<Json<Vec<RefundResponse>>, StatusCode>
where
    A: SessionService + 'static,
{
    let id = InvoiceId::from_string(invoice_id);

    // Verify invoice exists
    InvoiceReader::get(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let refunds = RefundReader::get_refunds_for_invoice(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(refunds.into_iter().map(Into::into).collect()))
}
