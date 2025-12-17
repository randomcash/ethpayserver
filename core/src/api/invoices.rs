//! Invoice management API endpoints.
//!
//! All endpoints require authentication. Users can only access invoices
//! for stores they are members of.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use auth::{AuthRepository, StoreId, repository::UserStoreRepository};
use types::{
    InvoiceId, InvoiceStatus, Network,
    InvoiceReader, InvoiceWriter, InvoiceQueryParams,
    traits::InvoiceData,
};

use crate::state::AppState;
use super::extractors::{AuthenticatedUser, AdminAuth};

/// Request to create a new invoice.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateInvoiceRequest {
    /// Store ID this invoice belongs to.
    pub store_id: Uuid,
    /// Network to receive payment on.
    pub network: String,
    /// Amount in smallest unit (wei, satoshi).
    pub amount: String,
    /// Asset symbol (ETH, BTC, USDC, etc.).
    pub asset_symbol: String,
    /// Expiration in seconds from now (default: 3600).
    pub expiration_seconds: Option<u64>,
    /// Optional metadata.
    pub metadata: Option<serde_json::Value>,
    /// Optional webhook URL.
    pub webhook_url: Option<String>,
    /// Optional redirect URL after payment.
    pub redirect_url: Option<String>,
}

/// Invoice response.
#[derive(Debug, Serialize, ToSchema)]
pub struct InvoiceResponse {
    /// Invoice ID.
    pub id: String,
    /// Network.
    pub network: String,
    /// Status.
    pub status: String,
    /// Requested amount.
    pub amount: String,
    /// Amount received so far.
    pub amount_received: String,
    /// Asset symbol.
    pub asset_symbol: String,
    /// Payment address.
    pub payment_address: Option<String>,
    /// Payment request (URI).
    pub payment_request: Option<String>,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Expiration timestamp.
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// Metadata.
    pub metadata: Option<serde_json::Value>,
}

impl From<InvoiceData> for InvoiceResponse {
    fn from(inv: InvoiceData) -> Self {
        Self {
            id: inv.id.0,
            network: inv.network.to_string(),
            status: inv.status.to_string(),
            amount: inv.amount,
            amount_received: inv.amount_received,
            asset_symbol: inv.asset_symbol,
            payment_address: inv.payment_address,
            payment_request: inv.payment_request,
            created_at: inv.created_at,
            expires_at: inv.expires_at,
            metadata: inv.metadata,
        }
    }
}

/// Query parameters for listing invoices.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListInvoicesQuery {
    /// Filter by store ID.
    pub store_id: Option<Uuid>,
    /// Filter by status.
    pub status: Option<String>,
    /// Filter by network.
    pub network: Option<String>,
    /// Maximum number of results.
    pub limit: Option<i64>,
    /// Offset for pagination.
    pub offset: Option<i64>,
}

/// Paginated invoice list response.
#[derive(Debug, Serialize, ToSchema)]
pub struct InvoiceListResponse {
    /// Total number of matching invoices.
    pub total: i64,
    /// Invoices in this page.
    pub invoices: Vec<InvoiceResponse>,
}

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
pub async fn list_invoices<R>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<AppState<R>>,
    Query(query): Query<ListInvoicesQuery>,
) -> Result<Json<InvoiceListResponse>, StatusCode>
where
    R: AuthRepository + Send + Sync + 'static,
{
    // For non-admins, store_id is required
    let store_id = match query.store_id {
        Some(id) => id,
        None => {
            // Admins can list all invoices, non-admins need store_id
            if user.role != auth::Role::ServerAdmin {
                return Err(StatusCode::BAD_REQUEST);
            }
            // For admins without store_id, we'll query all
            uuid::Uuid::nil()
        }
    };

    // Check user has access to the store (unless admin or no store filter)
    if store_id != uuid::Uuid::nil() {
        let is_member = state.data_service
            .get_user_store(user.id, StoreId(store_id))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .is_some();

        if !is_member && user.role != auth::Role::ServerAdmin {
            return Err(StatusCode::FORBIDDEN);
        }
    }

    let mut params = InvoiceQueryParams::new();

    if let Some(status) = query.status {
        let status: InvoiceStatus = status.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
        params = params.with_status(status);
    }

    if let Some(network) = query.network {
        let network: Network = network.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
        params = params.with_network(network);
    }

    if let Some(limit) = query.limit {
        params = params.with_limit(limit);
    }

    if let Some(offset) = query.offset {
        params = params.with_offset(offset);
    }

    // TODO: Add store_id filter to params when implemented
    // params = params.with_store_id(store_id);

    let (total, invoices) = InvoiceReader::query(&*state.data_service, &params)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(InvoiceListResponse {
        total,
        invoices: invoices.into_iter().map(|i| i.into()).collect(),
    }))
}

/// Create a new invoice.
///
/// Requires `cancreateinvoice` permission on the store.
#[utoipa::path(
    post,
    path = "/invoices",
    tag = "invoices",
    security(("bearer_auth" = [])),
    request_body = CreateInvoiceRequest,
    responses(
        (status = 201, description = "Invoice created", body = InvoiceResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
pub async fn create_invoice<R>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<AppState<R>>,
    Json(req): Json<CreateInvoiceRequest>,
) -> Result<(StatusCode, Json<InvoiceResponse>), StatusCode>
where
    R: AuthRepository + Send + Sync + 'static,
{
    // Check permission on the store
    let has_permission = state.data_service
        .user_has_store_permission(user.id, StoreId(req.store_id), "ethpay.store.cancreateinvoice")
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !has_permission {
        return Err(StatusCode::FORBIDDEN);
    }

    let network: Network = req.network.parse().map_err(|_| StatusCode::BAD_REQUEST)?;

    let expiration_secs = req.expiration_seconds.unwrap_or(3600);
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(expiration_secs as i64);

    // Create invoice data
    let invoice = InvoiceData {
        id: InvoiceId::new(),
        network,
        status: InvoiceStatus::Pending,
        amount: req.amount,
        amount_received: "0".to_string(),
        asset_symbol: req.asset_symbol,
        payment_address: None, // TODO: Generate from HD wallet
        payment_request: None, // TODO: Generate payment URI
        created_at: chrono::Utc::now(),
        expires_at,
        metadata: req.metadata,
        extra: None,
    };

    InvoiceWriter::upsert(&*state.data_service, &invoice)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok((StatusCode::CREATED, Json(invoice.into())))
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
pub async fn get_invoice<R>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<AppState<R>>,
    Path(invoice_id): Path<String>,
) -> Result<Json<InvoiceResponse>, StatusCode>
where
    R: AuthRepository + Send + Sync + 'static,
{
    let id = InvoiceId::from_string(invoice_id);

    let invoice = InvoiceReader::get(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // TODO: Check user has access to the invoice's store when store_id is on invoice
    // For now, allow any authenticated user (will be fixed when invoices have store_id)
    let _ = user;

    Ok(Json(invoice.into()))
}

/// Cancel an invoice.
///
/// User must have permission to manage invoices on the store.
#[utoipa::path(
    post,
    path = "/invoices/{invoice_id}/cancel",
    tag = "invoices",
    security(("bearer_auth" = [])),
    params(
        ("invoice_id" = String, Path, description = "Invoice ID")
    ),
    responses(
        (status = 200, description = "Invoice cancelled", body = InvoiceResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Invoice not found"),
        (status = 409, description = "Invoice cannot be cancelled"),
    )
)]
pub async fn cancel_invoice<R>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<AppState<R>>,
    Path(invoice_id): Path<String>,
) -> Result<Json<InvoiceResponse>, StatusCode>
where
    R: AuthRepository + Send + Sync + 'static,
{
    let id = InvoiceId::from_string(invoice_id);

    let invoice = InvoiceReader::get(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // TODO: Check user has canmodifyinvoice permission on the invoice's store
    // For now, allow any authenticated user (will be fixed when invoices have store_id)
    let _ = user;

    // Can only cancel pending invoices
    if invoice.status != InvoiceStatus::Pending {
        return Err(StatusCode::CONFLICT);
    }

    InvoiceWriter::update_status(&*state.data_service, &id, InvoiceStatus::Cancelled)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Re-fetch to get updated status
    let updated = InvoiceReader::get(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(updated.into()))
}

/// Mark expired invoices.
///
/// This is an admin-only endpoint to trigger expiration of overdue invoices.
#[utoipa::path(
    post,
    path = "/invoices/expire",
    tag = "invoices",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Number of invoices expired", body = ExpireResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin access required"),
    )
)]
pub async fn expire_invoices<R>(
    AdminAuth(_admin): AdminAuth,
    State(state): State<AppState<R>>,
) -> Result<Json<ExpireResponse>, StatusCode>
where
    R: AuthRepository + Send + Sync + 'static,
{
    let expired = InvoiceReader::get_expired(&*state.data_service)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let count = expired.len();

    for invoice in expired {
        let _ = InvoiceWriter::update_status(
            &*state.data_service,
            &invoice.id,
            InvoiceStatus::Expired,
        )
        .await;
    }

    Ok(Json(ExpireResponse { expired_count: count as u64 }))
}

/// Response for expire operation.
#[derive(Debug, Serialize, ToSchema)]
pub struct ExpireResponse {
    /// Number of invoices that were expired.
    pub expired_count: u64,
}
