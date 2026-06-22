use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use uuid::Uuid;

use ::types::{
    InvoiceId, InvoiceReader, InvoiceStatus, PaymentQueryParams, PaymentReader, StoreId,
};
use auth::{SessionService, repository::UserStoreRepository};
use data_service::PaymentOptionReader;

use super::{
    InvoiceStatusResponse, ListPaymentsQuery, PaymentListResponse, PaymentResponse,
    get_invoice_with_permission,
};
use crate::api::extractors::AuthenticatedUser;
use crate::state::PgAppState;

/// Get payments for an invoice.
///
/// User must be a member of the store the invoice belongs to.
#[utoipa::path(
    get,
    path = "/invoices/{invoice_id}/payments",
    tag = "invoices",
    security(("bearer_auth" = [])),
    params(
        ("invoice_id" = String, Path, description = "Invoice ID")
    ),
    responses(
        (status = 200, description = "List of payments", body = Vec<PaymentResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Invoice not found"),
    )
)]
pub async fn get_invoice_payments<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(invoice_id): Path<String>,
) -> Result<Json<Vec<PaymentResponse>>, StatusCode>
where
    A: SessionService + 'static,
{
    let id = InvoiceId::from_string(invoice_id);

    // Verify permission
    let _invoice = get_invoice_with_permission(&state, &user, &id).await?;

    let payments = PaymentReader::get_for_invoice(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(payments.into_iter().map(|p| p.into()).collect()))
}

/// List payments with optional filters.
///
/// Users can only see payments for stores they are members of.
/// store_id is required unless user is a server admin.
#[utoipa::path(
    get,
    path = "/payments",
    tag = "payments",
    security(("bearer_auth" = [])),
    params(ListPaymentsQuery),
    responses(
        (status = 200, description = "List of payments", body = PaymentListResponse),
        (status = 400, description = "store_id required"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Not a member of the store"),
    )
)]
pub async fn list_payments<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Query(query): Query<ListPaymentsQuery>,
) -> Result<Json<PaymentListResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    // For non-admins, store_id is required
    let store_id = match query.store_id {
        Some(id) => id,
        None => {
            if user.role != auth::Role::ServerAdmin {
                return Err(StatusCode::BAD_REQUEST);
            }
            uuid::Uuid::nil()
        }
    };

    // Check user has access to the store (unless admin or no store filter)
    if store_id != uuid::Uuid::nil() {
        let is_member = state
            .data_service
            .get_user_store(user.id, StoreId(store_id))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .is_some();

        if !is_member && user.role != auth::Role::ServerAdmin {
            return Err(StatusCode::FORBIDDEN);
        }
    }

    let mut params = PaymentQueryParams::new();

    if let Some(status) = query.status {
        match status.as_str() {
            "confirmed" => params = params.with_confirmed(true),
            "pending" => params = params.with_confirmed(false),
            _ => return Err(StatusCode::BAD_REQUEST),
        }
    }

    if let Some(limit) = query.limit {
        params = params.with_limit(limit);
    }

    if let Some(offset) = query.offset {
        params = params.with_offset(offset);
    }

    if store_id != uuid::Uuid::nil() {
        params = params.with_store_id(StoreId(store_id));
    }

    let (total, payments) = PaymentReader::query(&*state.data_service, &params)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(PaymentListResponse {
        total,
        payments: payments.into_iter().map(Into::into).collect(),
    }))
}

/// Get a single payment by ID.
///
/// User must be a member of the store the payment's invoice belongs to.
#[utoipa::path(
    get,
    path = "/payments/{payment_id}",
    tag = "payments",
    security(("bearer_auth" = [])),
    params(
        ("payment_id" = Uuid, Path, description = "Payment ID")
    ),
    responses(
        (status = 200, description = "Payment details", body = PaymentResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Payment not found"),
    )
)]
pub async fn get_payment<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(payment_id): Path<Uuid>,
) -> Result<Json<PaymentResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let payment = PaymentReader::get(&*state.data_service, payment_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Look up the invoice to check store membership
    let invoice = InvoiceReader::get(&*state.data_service, &payment.invoice_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if user.role != auth::Role::ServerAdmin {
        let is_member = state
            .data_service
            .get_user_store(user.id, invoice.store_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .is_some();

        if !is_member {
            return Err(StatusCode::NOT_FOUND);
        }
    }

    Ok(Json(payment.into()))
}

/// Get detailed status of an invoice including payments.
///
/// User must be a member of the store the invoice belongs to.
#[utoipa::path(
    get,
    path = "/invoices/{invoice_id}/status",
    tag = "invoices",
    security(("bearer_auth" = [])),
    params(
        ("invoice_id" = String, Path, description = "Invoice ID")
    ),
    responses(
        (status = 200, description = "Invoice status with payment details", body = InvoiceStatusResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Invoice not found"),
    )
)]
pub async fn get_invoice_status<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(invoice_id): Path<String>,
) -> Result<Json<InvoiceStatusResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let id = InvoiceId::from_string(invoice_id);

    let invoice = get_invoice_with_permission(&state, &user, &id).await?;

    let payments = PaymentReader::get_for_invoice(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let options = PaymentOptionReader::get_for_invoice(&*state.data_service, &id)
        .await
        .unwrap_or_default();

    let now = chrono::Utc::now();
    let confirmed_count = payments.iter().filter(|p| p.confirmed_at.is_some()).count();

    Ok(Json(InvoiceStatusResponse {
        id: invoice.id.0,
        status: invoice.status.to_string(),
        amount: invoice.amount.clone(),
        amount_received: invoice.amount_received,
        currency: invoice.currency,
        expires_at: invoice.expires_at,
        payment_count: payments.len(),
        confirmed_count,
        is_paid: invoice.status == InvoiceStatus::Paid,
        is_expired: invoice.status == InvoiceStatus::Expired || invoice.expires_at < now,
        payment_options: options.into_iter().map(Into::into).collect(),
        payments: payments.into_iter().map(|p| p.into()).collect(),
    }))
}
