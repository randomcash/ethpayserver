//! Refund API endpoints.
//!
//! POST /invoices/{invoice_id}/refund — Initiate a refund for a paid invoice.
//! GET  /invoices/{invoice_id}/refunds — List refunds for an invoice.
//!
//! The rule this module enforces: a payment can never be refunded for more than
//! it was worth, counting every refund already recorded against it.

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
use data_service::{InvoiceReader, PaymentReader, RefundReader, RefundWriter};
use types::{InvoiceId, InvoiceStatus, PaymentData, RefundData, RefundStatus};

use super::extractors::AuthenticatedUser;
use crate::metrics;
use crate::state::PgAppState;
pub use api_types::{CreateRefundRequest, RefundResponse};

/// Parse an amount held as a string of base units.
///
/// Digits and nothing else. Both parsers reachable from here read more than
/// that — `U256`'s `FromStr` takes `0x…`, so `"0x10"` would mean sixteen, and
/// `from_str_radix` takes `_` separators, so `"1_000"` would mean a thousand.
/// An amount in this system is a plain base-ten integer; a string that is
/// anything else is a mistake to reject, not a spelling to interpret.
fn parse_base_units(amount: &str) -> Option<U256> {
    if amount.is_empty() || !amount.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    U256::from_str_radix(amount, 10).ok()
}

/// How much of `payment` has already been refunded.
///
/// Failed refunds are excluded: nothing left the wallet, so that value is still
/// refundable. Pending and broadcasting refunds count — a refund in flight has
/// not failed yet, and treating it as free money is how the same payment gets
/// sent back twice.
///
/// A stored amount that will not parse returns `None` rather than being skipped:
/// skipping would undercount what has gone out and permit a refund on top of it.
fn already_refunded(refunds: &[RefundData], payment_id: Uuid) -> Option<U256> {
    refunds
        .iter()
        .filter(|r| r.payment_id == payment_id && r.status != RefundStatus::Failed)
        .try_fold(U256::ZERO, |acc, r| {
            parse_base_units(&r.amount).map(|amt| acc + amt)
        })
}

/// Decide what this refund request may pay out, in base units.
///
/// `requested` is the caller's optional amount; absent still means "the whole
/// payment", as the API has always documented. Neither was checked against
/// anything before: an arbitrary string went straight into the refund record, so
/// a caller could name any amount at all, in any format, and could do it
/// repeatedly.
///
/// - unparseable, zero, or larger than the payment itself → 400, the request is
///   wrong on its own terms
/// - within the payment but more than what is left after existing refunds → 409,
///   the request is well formed and the state refuses it
///
/// A partially refunded payment therefore answers 409 to an omitted amount
/// rather than quietly refunding the remainder: paying out less than the caller
/// asked for is not a correction to make on their behalf.
fn resolve_refund_amount(
    requested: Option<&str>,
    payment_amount: U256,
    already_refunded: U256,
) -> Result<U256, StatusCode> {
    let remaining = payment_amount.saturating_sub(already_refunded);

    let amount = match requested {
        Some(raw) => parse_base_units(raw).ok_or(StatusCode::BAD_REQUEST)?,
        None => payment_amount,
    };

    if amount.is_zero() || amount > payment_amount {
        return Err(StatusCode::BAD_REQUEST);
    }

    if amount > remaining {
        return Err(StatusCode::CONFLICT);
    }

    Ok(amount)
}

/// The amount this refund request may create, checked against the payment and
/// against every refund already recorded on it.
async fn refundable_amount<R>(
    reader: &R,
    invoice_id: &InvoiceId,
    payment: &PaymentData,
    requested: Option<&str>,
) -> Result<U256, StatusCode>
where
    R: RefundReader,
{
    let payment_amount = parse_base_units(&payment.amount).ok_or_else(|| {
        tracing::error!(
            payment_id = %payment.id,
            "Stored payment amount is not a base-ten integer; cannot bound a refund against it"
        );
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let existing = RefundReader::get_refunds_for_invoice(reader, invoice_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to read existing refunds");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let refunded = already_refunded(&existing, payment.id).ok_or_else(|| {
        tracing::error!(
            payment_id = %payment.id,
            "Stored refund amount is not a base-ten integer; refusing rather than undercounting"
        );
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    resolve_refund_amount(requested, payment_amount, refunded)
}

/// Initiate a refund for a paid invoice.
///
/// Validates the invoice is in Paid or LatePaid status, finds the confirmed
/// payment, checks the amount against what is left of it, and creates a refund
/// record. Nothing signs or broadcasts that refund: this deployment holds no
/// spending key, so the record stays `Pending` until something outside this
/// process acts on it.
pub async fn create_refund<A>(
    AuthenticatedUser(user): AuthenticatedUser,
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

    // Verify user has access to this invoice's store
    if !user.role.is_admin()
        && state
            .data_service
            .get_user_store(user.id, invoice.store_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }

    if !matches!(
        invoice.status,
        InvoiceStatus::Paid | InvoiceStatus::LatePaid
    ) {
        tracing::warn!(invoice_id = %id.0, status = %invoice.status, "Cannot refund invoice in this status");
        return Err(StatusCode::BAD_REQUEST);
    }

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

    let refund_amount =
        refundable_amount(&*state.data_service, &id, &payment, body.amount.as_deref()).await?;

    // Create refund record
    let refund = RefundData {
        id: Uuid::new_v4(),
        invoice_id: id.clone(),
        payment_id: payment.id,
        store_id: invoice.store_id,
        to_address,
        chain_id: payment.chain_id.clone(),
        asset_type: payment.asset_type.to_string(),
        asset_symbol: payment.asset_symbol.clone(),
        token_address: payment.token_address.clone(),
        amount: refund_amount.to_string(),
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

    metrics::record_refund_initiated(&payment.chain_id, &payment.asset_symbol);
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
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(invoice_id): Path<String>,
) -> Result<Json<Vec<RefundResponse>>, StatusCode>
where
    A: SessionService + 'static,
{
    let id = InvoiceId::from_string(invoice_id);

    // Verify invoice exists
    let invoice = InvoiceReader::get(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Verify user has access to this invoice's store
    if !user.role.is_admin()
        && state
            .data_service
            .get_user_store(user.id, invoice.store_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }

    let refunds = RefundReader::get_refunds_for_invoice(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(refunds.into_iter().map(Into::into).collect()))
}
