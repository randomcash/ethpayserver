use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};

use ::types::{InvoiceId, InvoiceQueryParams, InvoiceReader, InvoiceStatus};
use auth::{SessionService, repository::UserStoreRepository};
use data_service::PaymentOptionReader;

use crate::api::extractors::AuthenticatedUser;
use crate::state::PgAppState;

use super::{
    InvoiceListResponse, InvoiceResponse, ListInvoicesQuery, extract_customer_email,
    verify_store_access_for_query,
};

/// List invoices with optional filters.
///
/// Users can only see invoices for stores they are members of.
/// store_id is required unless user is a server admin.
#[utoipa::path(
    get,
    path = "/invoices",
    tag = "invoices",
    security(("bearer_auth" = [])),
    params(ListInvoicesQuery),
    responses(
        (status = 200, description = "List of invoices", body = InvoiceListResponse),
        (status = 400, description = "store_id required"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Not a member of the store"),
    )
)]
pub async fn list_invoices<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Query(query): Query<ListInvoicesQuery>,
) -> Result<Json<InvoiceListResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    // Resolve the store scope once. `Some` is membership-checked, `None` means
    // every store and is admin-only. See verify_store_access_for_query - the
    // Option is load-bearing, a nil-UUID sentinel here was RCS-211.
    let store_id =
        verify_store_access_for_query(&*state.data_service, &user, query.store_id).await?;

    let mut params = InvoiceQueryParams::new();

    if let Some(status) = query.status {
        let status: InvoiceStatus = status.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
        params = params.with_status(status);
    }

    if let Some(currency) = query.currency {
        params = params.with_currency(currency);
    }

    if let Some(limit) = query.limit {
        params = params.with_limit(limit);
    }

    if let Some(offset) = query.offset {
        params = params.with_offset(offset);
    }

    // `None` is an admin querying every store, so no store filter is applied.
    if let Some(store_id) = store_id {
        params = params.with_store_id(store_id);
    }

    let (total, invoices) = InvoiceReader::query(&*state.data_service, &params)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Get payment options for each invoice
    let mut responses = Vec::with_capacity(invoices.len());
    for invoice in invoices {
        let options = PaymentOptionReader::get_for_invoice(&*state.data_service, &invoice.id)
            .await
            .unwrap_or_default();

        let customer_email = extract_customer_email(&invoice.metadata);
        responses.push(InvoiceResponse {
            id: invoice.id.0,
            currency: invoice.currency,
            status: invoice.status.to_string(),
            amount: invoice.amount,
            amount_received: invoice.amount_received,
            created_at: invoice.created_at,
            expires_at: invoice.expires_at,
            metadata: invoice.metadata,
            customer_email,
            payment_options: options.into_iter().map(Into::into).collect(),
        });
    }

    Ok(Json(InvoiceListResponse {
        total,
        invoices: responses,
    }))
}

/// Get an invoice by ID.
///
/// User must be a member of the store the invoice belongs to.
#[utoipa::path(
    get,
    path = "/invoices/{invoice_id}",
    tag = "invoices",
    security(("bearer_auth" = [])),
    params(
        ("invoice_id" = String, Path, description = "Invoice ID")
    ),
    responses(
        (status = 200, description = "Invoice details", body = InvoiceResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Not a member of the invoice's store"),
        (status = 404, description = "Invoice not found"),
    )
)]
pub async fn get_invoice<A>(
    AuthenticatedUser(user): AuthenticatedUser,
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

    // Check user has access to the invoice's store (unless admin)
    if user.role != auth::Role::ServerAdmin {
        let is_member = state
            .data_service
            .get_user_store(user.id, invoice.store_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .is_some();

        if !is_member {
            return Err(StatusCode::FORBIDDEN);
        }
    }

    let options = PaymentOptionReader::get_for_invoice(&*state.data_service, &id)
        .await
        .unwrap_or_default();

    let customer_email = extract_customer_email(&invoice.metadata);
    let response = InvoiceResponse {
        id: invoice.id.0,
        currency: invoice.currency,
        status: invoice.status.to_string(),
        amount: invoice.amount,
        amount_received: invoice.amount_received,
        created_at: invoice.created_at,
        expires_at: invoice.expires_at,
        metadata: invoice.metadata,
        customer_email,
        payment_options: options.into_iter().map(Into::into).collect(),
    };

    Ok(Json(response))
}
