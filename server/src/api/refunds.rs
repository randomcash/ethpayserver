//! Refund API endpoints.
//!
//! POST /invoices/{invoice_id}/refund — refuses; refunds are the merchant's job.
//! GET  /invoices/{invoice_id}/refunds — List refunds for an invoice.
//!
//! # Refunds are the merchant's job (RCS-272)
//!
//! This deployment is non-custodial: it derives payment addresses from a
//! merchant's xpub and never holds the matching private key (see
//! `evm::wallet::validate_xpub` and `README.md`), so it cannot sign or
//! broadcast a transaction that would send money anywhere, refund included.
//!
//! `create_refund` used to accept this request, check the amount against the
//! payment, and write a `Pending` refund row — and stop there, because nothing
//! downstream of it could ever sign the transaction. Nothing ever moved that
//! row past `Pending`, so a merchant who called it got a refund that stayed
//! "pending" forever, indistinguishable from one about to happen. That is
//! worse than refusing outright: it presents a capability the server does not
//! have. The endpoint now says so, and creates nothing.
//!
//! A merchant refunds a payer from their own wallet, the one that holds the
//! spending key. This endpoint remains only as an explicit, documented refusal
//! rather than a route that disappears with no explanation.
//!
//! ## Existing testnet rows
//!
//! Checked directly against the testnet database on 2026-09-14: the
//! `refunds` table held zero rows of any status, `Pending`/`Broadcasting`
//! included. Nothing there was ever real, so this change ships with no
//! migration and no backfill — there is nothing to migrate away from. If
//! that ever stops being true (a restored backup, a different environment),
//! treat any `Pending`/`Broadcasting` row found there the same way: it is a
//! record of a request this server was never able to carry out, not a
//! refund in progress.

#[cfg(test)]
mod tests;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use auth::{SessionService, UserStoreRepository};
use data_service::{InvoiceReader, RefundReader};
use types::InvoiceId;

use super::extractors::AuthenticatedUser;
use crate::api::ApiErr;
use crate::state::PgAppState;
pub use api_types::RefundResponse;

/// Why `POST /invoices/{id}/refund` always refuses. See the module docs.
const REFUND_UNSUPPORTED_REASON: &str = "refunds are the merchant's responsibility: this server holds no spending key and cannot send funds. Refund the payer from your own wallet.";

/// The fixed response every refund request gets, regardless of the invoice,
/// its status, or the caller's amount. Nothing here can ever act on a refund,
/// so nothing here is worth validating first.
fn refund_unsupported() -> ApiErr {
    (
        StatusCode::NOT_IMPLEMENTED,
        REFUND_UNSUPPORTED_REASON.to_string(),
    )
        .into()
}

/// Refuse to create a refund. See the module docs: refunds are the merchant's
/// job, done from their own wallet, because this server holds no spending key.
pub async fn create_refund<A>(
    AuthenticatedUser(_user): AuthenticatedUser,
    State(_state): State<PgAppState<A>>,
    Path(_invoice_id): Path<String>,
) -> ApiErr
where
    A: SessionService + 'static,
{
    refund_unsupported()
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
