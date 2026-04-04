//! Dashboard statistics endpoint.
//!
//! Returns aggregated stats for the authenticated user's stores.

use axum::{Json, extract::State, http::StatusCode};
use serde::Serialize;
use utoipa::ToSchema;

use auth::{SessionService, repository::StoreRepository};
use data_service::{InvoiceQueryParams, InvoiceReader, PaymentQueryParams, PaymentReader};
use types::InvoiceStatus;

use super::extractors::AuthenticatedUser;
use crate::state::PgAppState;

/// Dashboard statistics response.
#[derive(Debug, Serialize, ToSchema)]
pub struct DashboardStats {
    /// Total number of invoices across all user stores.
    pub total_invoices: i64,
    /// Number of pending invoices.
    pub pending_invoices: i64,
    /// Number of paid invoices.
    pub paid_invoices: i64,
    /// Number of expired invoices.
    pub expired_invoices: i64,
    /// Total number of payments received.
    pub total_payments: i64,
    /// Number of stores the user has access to.
    pub total_stores: u32,
}

/// Get dashboard statistics for the authenticated user.
#[utoipa::path(
    get,
    path = "/dashboard/stats",
    tag = "dashboard",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Dashboard statistics", body = DashboardStats),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn get_stats<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
) -> Result<Json<DashboardStats>, StatusCode>
where
    A: SessionService + 'static,
{
    let ds = &*state.data_service;

    // Get user's stores
    let stores: Vec<auth::Store> = ds
        .get_stores_for_user(user.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let total_stores = stores.len() as u32;

    let mut total_invoices: i64 = 0;
    let mut pending_invoices: i64 = 0;
    let mut paid_invoices: i64 = 0;
    let mut expired_invoices: i64 = 0;
    let mut total_payments: i64 = 0;

    for store in &stores {
        let store_id = auth::StoreId(store.id.0);

        // Total invoices
        let (count, _) = InvoiceReader::query(
            ds,
            &InvoiceQueryParams::new()
                .with_store_id(store_id)
                .with_limit(0),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        total_invoices += count;

        // Pending
        let (count, _) = InvoiceReader::query(
            ds,
            &InvoiceQueryParams::new()
                .with_store_id(store_id)
                .with_status(InvoiceStatus::Pending)
                .with_limit(0),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        pending_invoices += count;

        // Paid
        let (count, _) = InvoiceReader::query(
            ds,
            &InvoiceQueryParams::new()
                .with_store_id(store_id)
                .with_status(InvoiceStatus::Paid)
                .with_limit(0),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        paid_invoices += count;

        // Expired
        let (count, _) = InvoiceReader::query(
            ds,
            &InvoiceQueryParams::new()
                .with_store_id(store_id)
                .with_status(InvoiceStatus::Expired)
                .with_limit(0),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        expired_invoices += count;

        // Payments
        let (count, _) = PaymentReader::query(
            ds,
            &PaymentQueryParams::new()
                .with_store_id(store_id)
                .with_limit(0),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        total_payments += count;
    }

    Ok(Json(DashboardStats {
        total_invoices,
        pending_invoices,
        paid_invoices,
        expired_invoices,
        total_payments,
        total_stores,
    }))
}
