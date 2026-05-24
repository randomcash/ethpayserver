use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use ::types::InvoiceReader;
use auth::{SessionService, repository::UserStoreRepository};
use data_service::PaymentOptionReader;

use super::{InvoiceResponse, TxHashLookupResponse, extract_customer_email};
use crate::api::extractors::AuthenticatedUser;
use crate::state::PgAppState;

/// Validate a tx hash: must be 0x followed by 64 hex characters.
pub(crate) fn is_valid_tx_hash(hash: &str) -> bool {
    hash.len() == 66 && hash.starts_with("0x") && hash[2..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Path parameters for tx hash lookup.
#[derive(Debug, Deserialize)]
pub struct TxHashLookupPath {
    pub chain_id: u64,
    pub tx_hash: String,
}

/// Lookup an invoice by transaction hash.
///
/// Finds the payment matching the given chain_id and tx_hash, then returns
/// the associated invoice. Returns 404 if no match or if the caller doesn't
/// own the invoice's store (prevents enumeration).
#[utoipa::path(
    get,
    path = "/invoices/by-tx/{chain_id}/{tx_hash}",
    tag = "invoices",
    security(("bearer_auth" = [])),
    params(
        ("chain_id" = u64, Path, description = "EIP-155 chain ID"),
        ("tx_hash" = String, Path, description = "Transaction hash (0x-prefixed, 64 hex chars)")
    ),
    responses(
        (status = 200, description = "Invoice and payment found", body = TxHashLookupResponse),
        (status = 400, description = "Malformed tx_hash"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "No invoice found for this transaction"),
    )
)]
pub async fn lookup_by_tx_hash<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(path): Path<TxHashLookupPath>,
) -> Result<Json<TxHashLookupResponse>, (StatusCode, Json<serde_json::Value>)>
where
    A: SessionService + 'static,
{
    // Validate tx_hash format
    if !is_valid_tx_hash(&path.tx_hash) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "malformed_tx_hash"})),
        ));
    }

    // Look up payment by (chain_id, tx_hash)
    let payment = state
        .data_service
        .get_payment_by_tx_hash(path.chain_id, &path.tx_hash)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal_error"})),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "not_found"})),
            )
        })?;

    // Fetch the linked invoice
    let invoice = InvoiceReader::get(&*state.data_service, &payment.invoice_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal_error"})),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "not_found"})),
            )
        })?;

    // Permission check — non-admins must own the store (return 404 to hide existence)
    if user.role != auth::Role::ServerAdmin {
        let is_member = state
            .data_service
            .get_user_store(user.id, invoice.store_id)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": "internal_error"})),
                )
            })?
            .is_some();

        if !is_member {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "not_found"})),
            ));
        }
    }

    // Get payment options for the invoice
    let options = PaymentOptionReader::get_for_invoice(&*state.data_service, &invoice.id)
        .await
        .unwrap_or_default();

    let customer_email = extract_customer_email(&invoice.metadata);
    let response = TxHashLookupResponse {
        invoice: InvoiceResponse {
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
        },
        payment: payment.into(),
    };

    Ok(Json(response))
}
