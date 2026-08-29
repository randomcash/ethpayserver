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

use auth::{SessionService, UserStoreRepository};
use data_service::{InvoiceReader, PaymentReader, RefundReader, RefundWriter};
use types::{InvoiceId, InvoiceStatus, PaymentData, RefundData, RefundStatus};

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

// ============================================================================
// Refund decision logic
//
// Extracted from `create_refund` so the validation and selection rules can be
// unit tested without a live `PgAppState` (which is bound to a real Postgres
// pool). The handler stays the only place that emits logs / touches the DB.
//
// These are `pub` so the end-to-end refund flow test (`tests/refund_flow.rs`)
// drives the same rules the handler does, rather than a copy of them.
// ============================================================================

/// Whether an invoice in `status` is eligible for a refund.
///
/// Only an invoice that actually received funds can be refunded, so a refund
/// is limited to `Paid` and `LatePaid`.
pub fn is_refundable_status(status: InvoiceStatus) -> bool {
    matches!(status, InvoiceStatus::Paid | InvoiceStatus::LatePaid)
}

/// Pick the payment a refund should be issued against.
///
/// Only a confirmed, non-reorged payment is refundable: an unconfirmed payment
/// may never land, and a reorged one has been rolled back on-chain. Returns the
/// first match in the order the reader supplied.
pub fn select_refundable_payment(payments: Vec<PaymentData>) -> Option<PaymentData> {
    payments
        .into_iter()
        .find(|p| p.confirmed_at.is_some() && !p.reorged)
}

/// Resolve the amount to refund.
///
/// An explicit `requested` amount makes this a partial refund; omitting it
/// refunds the full payment.
pub fn resolve_refund_amount(requested: Option<String>, payment_amount: &str) -> String {
    requested.unwrap_or_else(|| payment_amount.to_string())
}

/// Initiate a refund for a paid invoice.
///
/// Validates the invoice is in Paid or LatePaid status, finds the confirmed
/// payment, and creates a refund record. The actual transaction signing and
/// broadcasting is handled by a background service.
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

    if !is_refundable_status(invoice.status) {
        tracing::warn!(invoice_id = %id.0, status = %invoice.status, "Cannot refund invoice in this status");
        return Err(StatusCode::BAD_REQUEST);
    }

    let payments = PaymentReader::get_valid_for_invoice(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let payment = select_refundable_payment(payments).ok_or_else(|| {
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

    let refund_amount = resolve_refund_amount(body.amount, &payment.amount);

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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    use types::{AssetType, StoreId};

    const TEST_CHAIN_ID: u64 = 11155111;
    const PAYER: &str = "0xabcdef1234567890abcdef1234567890abcdef12";

    /// A confirmed, non-reorged payment — the refundable baseline that the
    /// individual tests mutate one field at a time.
    fn confirmed_payment(invoice_id: &InvoiceId) -> PaymentData {
        PaymentData {
            id: Uuid::new_v4(),
            invoice_id: invoice_id.clone(),
            payment_option_id: None,
            chain_id: TEST_CHAIN_ID,
            asset_type: AssetType::Native,
            amount: "1000000000000000000".to_string(),
            asset_symbol: "ETH".to_string(),
            token_address: None,
            tx_hash: format!("0x{:064x}", Uuid::new_v4().as_u128()),
            block_number: Some(12_345_678),
            detected_at: Utc::now(),
            confirmed_at: Some(Utc::now()),
            from_address: Some(PAYER.to_string()),
            reorged: false,
            extra: None,
            credited_amount: None,
            rate_used: None,
            rate_applied_at: None,
        }
    }

    fn test_refund(invoice_id: &InvoiceId, status: RefundStatus) -> RefundData {
        RefundData {
            id: Uuid::new_v4(),
            invoice_id: invoice_id.clone(),
            payment_id: Uuid::new_v4(),
            store_id: StoreId::new(),
            to_address: PAYER.to_string(),
            chain_id: TEST_CHAIN_ID,
            asset_type: "native".to_string(),
            asset_symbol: "ETH".to_string(),
            token_address: None,
            amount: "1000000000000000000".to_string(),
            tx_hash: None,
            status,
            fee_amount: None,
            reason: None,
            error_message: None,
            created_at: Utc::now(),
            confirmed_at: None,
        }
    }

    // ------------------------------------------------------------------
    // Refund validation — which invoices may be refunded
    // ------------------------------------------------------------------

    #[test]
    fn only_paid_and_late_paid_invoices_are_refundable() {
        assert!(is_refundable_status(InvoiceStatus::Paid));
        assert!(is_refundable_status(InvoiceStatus::LatePaid));
    }

    /// Refunding an invoice that never received funds would send money the
    /// merchant was never paid, so every other status must be rejected.
    #[test]
    fn unfunded_and_terminal_invoice_statuses_are_not_refundable() {
        for status in [
            InvoiceStatus::Pending,
            InvoiceStatus::Processing,
            InvoiceStatus::PartiallyPaid,
            InvoiceStatus::Expired,
            InvoiceStatus::Cancelled,
            InvoiceStatus::Refunded,
        ] {
            assert!(
                !is_refundable_status(status),
                "{status} must not be refundable"
            );
        }
    }

    // ------------------------------------------------------------------
    // Refund validation — which payment a refund is issued against
    // ------------------------------------------------------------------

    #[test]
    fn selects_a_confirmed_payment() {
        let invoice_id = InvoiceId::new();
        let payment = confirmed_payment(&invoice_id);
        let expected_id = payment.id;

        let selected = select_refundable_payment(vec![payment]).expect("payment is refundable");
        assert_eq!(selected.id, expected_id);
    }

    #[test]
    fn no_payments_means_nothing_to_refund() {
        assert!(select_refundable_payment(vec![]).is_none());
    }

    /// An unconfirmed payment may still never land on-chain — refunding it
    /// would pay out against funds the merchant does not hold.
    #[test]
    fn unconfirmed_payments_are_not_refundable() {
        let invoice_id = InvoiceId::new();
        let mut payment = confirmed_payment(&invoice_id);
        payment.confirmed_at = None;

        assert!(select_refundable_payment(vec![payment]).is_none());
    }

    /// A reorged payment was rolled back on-chain, so the funds are gone even
    /// though the row still carries a `confirmed_at`.
    #[test]
    fn reorged_payments_are_not_refundable() {
        let invoice_id = InvoiceId::new();
        let mut payment = confirmed_payment(&invoice_id);
        payment.reorged = true;

        assert!(select_refundable_payment(vec![payment]).is_none());
    }

    #[test]
    fn skips_unusable_payments_and_picks_the_confirmed_one() {
        let invoice_id = InvoiceId::new();

        let mut reorged = confirmed_payment(&invoice_id);
        reorged.reorged = true;

        let mut unconfirmed = confirmed_payment(&invoice_id);
        unconfirmed.confirmed_at = None;

        let good = confirmed_payment(&invoice_id);
        let expected_id = good.id;

        let selected = select_refundable_payment(vec![reorged, unconfirmed, good])
            .expect("the confirmed payment is refundable");
        assert_eq!(selected.id, expected_id);
    }

    // ------------------------------------------------------------------
    // Refund validation — amount resolution
    // ------------------------------------------------------------------

    #[test]
    fn omitting_the_amount_refunds_the_full_payment() {
        assert_eq!(
            resolve_refund_amount(None, "1000000000000000000"),
            "1000000000000000000"
        );
    }

    #[test]
    fn an_explicit_amount_makes_it_a_partial_refund() {
        assert_eq!(
            resolve_refund_amount(Some("250000000000000000".to_string()), "1000000000000000000"),
            "250000000000000000"
        );
    }

    /// Documents current behaviour, which is NOT a safety guarantee: the
    /// handler forwards the requested amount verbatim, so a request may ask
    /// for more than was actually paid. Bounding it against the payment is a
    /// behaviour change and is tracked separately — see the RCS-144 PR notes.
    #[test]
    fn requested_amount_is_not_currently_capped_at_the_payment_amount() {
        assert_eq!(
            resolve_refund_amount(Some("9".repeat(30)), "1000000000000000000"),
            "9".repeat(30)
        );
    }

    // ------------------------------------------------------------------
    // Refund status transitions, as seen through the API response
    // ------------------------------------------------------------------

    /// The wire format must stay snake_case: dashboards and merchant
    /// integrations match on these exact strings.
    #[test]
    fn response_serialises_each_refund_status() {
        let invoice_id = InvoiceId::new();
        for (status, expected) in [
            (RefundStatus::Pending, "pending"),
            (RefundStatus::Broadcasting, "broadcasting"),
            (RefundStatus::Confirmed, "confirmed"),
            (RefundStatus::Failed, "failed"),
        ] {
            let response = RefundResponse::from(test_refund(&invoice_id, status));
            assert_eq!(response.status, expected);
        }
    }

    #[test]
    fn pending_and_broadcasting_are_not_final_but_confirmed_and_failed_are() {
        assert!(!RefundStatus::Pending.is_final());
        assert!(!RefundStatus::Broadcasting.is_final());
        assert!(RefundStatus::Confirmed.is_final());
        assert!(RefundStatus::Failed.is_final());
    }

    /// The lifecycle a refund walks: created pending, broadcast, then settled.
    #[test]
    fn refund_lifecycle_reaches_a_final_status() {
        let lifecycle = [
            RefundStatus::Pending,
            RefundStatus::Broadcasting,
            RefundStatus::Confirmed,
        ];

        let (last, leading) = lifecycle.split_last().unwrap();
        for status in leading {
            assert!(!status.is_final(), "{status} is mid-flight");
        }
        assert!(last.is_final(), "the lifecycle ends in a final status");
    }

    #[test]
    fn refund_status_round_trips_through_its_wire_string() {
        for status in [
            RefundStatus::Pending,
            RefundStatus::Broadcasting,
            RefundStatus::Confirmed,
            RefundStatus::Failed,
        ] {
            let parsed: RefundStatus = status.as_str().parse().unwrap();
            assert_eq!(parsed, status);
        }
    }

    // ------------------------------------------------------------------
    // Response mapping
    // ------------------------------------------------------------------

    /// A freshly created refund is pending with no transaction yet — the
    /// broadcasting service fills `tx_hash`/`fee_amount`/`confirmed_at` later.
    #[test]
    fn newly_created_refund_maps_to_a_pending_response() {
        let invoice_id = InvoiceId::new();
        let mut refund = test_refund(&invoice_id, RefundStatus::Pending);
        refund.reason = Some("customer request".to_string());

        let response = RefundResponse::from(refund.clone());

        assert_eq!(response.id, refund.id);
        assert_eq!(response.invoice_id, invoice_id.0);
        assert_eq!(response.payment_id, refund.payment_id);
        assert_eq!(response.to_address, PAYER);
        assert_eq!(response.chain_id, TEST_CHAIN_ID);
        assert_eq!(response.asset_type, "native");
        assert_eq!(response.asset_symbol, "ETH");
        assert_eq!(response.amount, "1000000000000000000");
        assert_eq!(response.status, "pending");
        assert_eq!(response.reason.as_deref(), Some("customer request"));
        assert!(response.tx_hash.is_none());
        assert!(response.fee_amount.is_none());
        assert!(response.error_message.is_none());
        assert!(response.confirmed_at.is_none());
    }

    #[test]
    fn confirmed_refund_response_carries_the_transaction_details() {
        let invoice_id = InvoiceId::new();
        let mut refund = test_refund(&invoice_id, RefundStatus::Confirmed);
        refund.tx_hash = Some("0xdeadbeef".to_string());
        refund.fee_amount = Some("21000".to_string());
        refund.confirmed_at = Some(Utc::now());

        let response = RefundResponse::from(refund);

        assert_eq!(response.status, "confirmed");
        assert_eq!(response.tx_hash.as_deref(), Some("0xdeadbeef"));
        assert_eq!(response.fee_amount.as_deref(), Some("21000"));
        assert!(response.confirmed_at.is_some());
    }

    #[test]
    fn failed_refund_response_carries_the_error_message() {
        let invoice_id = InvoiceId::new();
        let mut refund = test_refund(&invoice_id, RefundStatus::Failed);
        refund.error_message = Some("insufficient balance".to_string());

        let response = RefundResponse::from(refund);

        assert_eq!(response.status, "failed");
        assert_eq!(
            response.error_message.as_deref(),
            Some("insufficient balance")
        );
        assert!(response.confirmed_at.is_none());
    }
}
