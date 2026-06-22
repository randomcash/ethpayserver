use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use ::types::{InvoiceId, InvoiceReader, InvoiceStatus, InvoiceWriter};
use auth::SessionService;
use data_service::{PaymentOptionReader, PaymentOptionWriter};

use crate::api::extractors::AdminAuth;
use crate::state::PgAppState;

use super::{InvoiceResponse, extract_customer_email};

/// Cancel an invoice (admin only).
///
/// Only works for pending/processing invoices.
#[utoipa::path(
    post,
    path = "/admin/invoices/{invoice_id}/cancel",
    tag = "admin",
    security(("bearer_auth" = [])),
    params(
        ("invoice_id" = String, Path, description = "Invoice ID")
    ),
    responses(
        (status = 200, description = "Invoice cancelled", body = InvoiceResponse),
        (status = 400, description = "Cannot cancel - already paid/cancelled"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Not an admin"),
        (status = 404, description = "Invoice not found"),
    )
)]
pub async fn cancel_invoice<A>(
    AdminAuth(_admin): AdminAuth,
    State(state): State<PgAppState<A>>,
    Path(invoice_id): Path<String>,
) -> Result<Json<InvoiceResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let id = InvoiceId::from_string(invoice_id);

    let invoice = InvoiceReader::get(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Can only cancel pending/processing invoices
    match invoice.status {
        InvoiceStatus::Pending | InvoiceStatus::Processing | InvoiceStatus::PartiallyPaid => {}
        _ => return Err(StatusCode::BAD_REQUEST),
    }

    InvoiceWriter::update_status(&*state.data_service, &id, InvoiceStatus::Cancelled)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Deactivate payment options
    if let Err(e) = PaymentOptionWriter::deactivate_for_invoice(&*state.data_service, &id).await {
        tracing::warn!(invoice_id = %id.0, error = %e, "Failed to deactivate payment options");
    }

    let mut cancelled = invoice;
    cancelled.status = InvoiceStatus::Cancelled;

    let options = PaymentOptionReader::get_for_invoice(&*state.data_service, &id)
        .await
        .unwrap_or_default();

    let customer_email = extract_customer_email(&cancelled.metadata);
    let response = InvoiceResponse {
        id: cancelled.id.0,
        currency: cancelled.currency,
        status: cancelled.status.to_string(),
        amount: cancelled.amount,
        amount_received: cancelled.amount_received,
        created_at: cancelled.created_at,
        expires_at: cancelled.expires_at,
        metadata: cancelled.metadata,
        customer_email,
        payment_options: options.into_iter().map(Into::into).collect(),
    };

    // Record metrics
    crate::metrics::record_invoice_cancelled();

    Ok(Json(response))
}
