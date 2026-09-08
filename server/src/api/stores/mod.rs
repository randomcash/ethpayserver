//! Store management API endpoints.
//!
//! All endpoints require authentication. Store-level permissions are checked
//! for operations on specific stores.

mod crud;
mod members;
mod payment_methods;
mod settings;
mod token_policy;
mod wallets;
mod webhooks;

// Re-export all handlers and types for route registration.
pub use crud::*;
pub use members::*;
pub use payment_methods::*;
pub use settings::*;
pub use token_policy::*;
pub use wallets::*;
pub use webhooks::*;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use auth::SessionService;
use auth::repository::UserStoreRepository;

use crate::state::PgAppState;

/// Re-exported so this crate has exactly one masking rule. It used to keep its
/// own byte-identical copy, so a single `RotateWalletResponse` could have
/// carried two different rules the moment either changed.
pub(crate) use api_types::mask_xpub;

/// A status, optionally with a reason the caller can read.
///
/// Handlers in this module mostly return bare `StatusCode`, and that stays
/// true: `From<StatusCode>` gives an empty reason, so `?` on the existing
/// permission and lookup helpers is unchanged and those responses keep exactly
/// the shape they had. What this adds is somewhere for a repository error's own
/// words to travel, for the cases where the status alone does not say enough.
pub struct ApiErr(StatusCode, String);

impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        // An empty reason stays a bare status, which is what every handler here
        // returned before and what `From<StatusCode>` produces.
        //
        // Note what this does NOT fix: both branches send an empty body, so the
        // client still Displays a reasonless error as "HTTP error 404: ",
        // trailing colon and all. The difference is only that the bare branch
        // sends no `content-type` for a body that does not exist. Filling the
        // reason is what removes the colon, and that is the caller's job - see
        // `repository_error`, which does it for the two variants that have
        // something worth saying.
        if self.1.is_empty() {
            self.0.into_response()
        } else {
            (self.0, self.1).into_response()
        }
    }
}

impl From<StatusCode> for ApiErr {
    fn from(status: StatusCode) -> Self {
        Self(status, String::new())
    }
}

impl From<(StatusCode, String)> for ApiErr {
    fn from((status, reason): (StatusCode, String)) -> Self {
        Self(status, reason)
    }
}

/// The status a repository error deserves, and a reason the caller can read.
///
/// `Conflict` has to survive the trip: since RCS-234 an xpub already registered
/// to another account is refused, and that refusal is reachable from the
/// ordinary payment-method form, not just from `POST /wallets`. Collapsing
/// every repository error into a 500 turned a merchant pasting the wrong key
/// into an opaque server error with nothing to act on.
///
/// The reason travels with it, because a bare status is barely better. The
/// client renders the response body after the code (`ApiError::Http`), so a
/// 409 with no body reaches the merchant as the literal string
/// "HTTP error 409:" - a dead end with the one useful word missing. That is
/// what the payment-method form actually showed.
///
/// Only `Conflict` and `NotFound` carry their message out. Those two are
/// written by this codebase for exactly this purpose; everything else may hold
/// a database error, whose text can name columns, constraints and hosts, and
/// none of that belongs in a response.
pub fn repository_error(err: data_service::RepositoryError) -> ApiErr {
    match err {
        data_service::RepositoryError::Conflict(msg) => ApiErr(StatusCode::CONFLICT, msg),
        data_service::RepositoryError::NotFound(msg) => ApiErr(StatusCode::NOT_FOUND, msg),
        _ => ApiErr(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal error".to_string(),
        ),
    }
}

/// Check if user can modify store settings (admin or has permission).
pub(crate) async fn require_store_settings_permission<A: SessionService>(
    state: &PgAppState<A>,
    user: &auth::UserInfo,
    store_id: uuid::Uuid,
) -> Result<(), StatusCode> {
    if user.role == auth::Role::ServerAdmin {
        return Ok(());
    }

    let has_permission = state
        .data_service
        .user_has_store_permission(
            user.id,
            auth::StoreId(store_id),
            "ethpay.store.canmodifystoresettings",
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if has_permission {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

#[cfg(test)]
mod tests;
