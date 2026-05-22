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

use auth::SessionService;
use auth::repository::UserStoreRepository;

use crate::state::PgAppState;

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

/// Mask an xpub for display (show first 8 and last 8 chars).
pub(crate) fn mask_xpub(xpub: &str) -> String {
    if xpub.len() <= 20 {
        return "****".to_string();
    }
    format!("{}...{}", &xpub[..8], &xpub[xpub.len() - 8..])
}

#[cfg(test)]
mod tests;
