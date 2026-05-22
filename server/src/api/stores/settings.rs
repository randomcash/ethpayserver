//! Store settings endpoints: get and update (PATCH).

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use auth::repository::StoreRepository;
use auth::{SessionService, StoreId};

use super::super::extractors::AuthenticatedUser;
use super::require_store_settings_permission;
use crate::state::PgAppState;

/// Store settings response.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct StoreSettingsResponse {
    pub store_id: Uuid,
    pub default_chain_id: Option<i64>,
    pub default_display_currency: Option<String>,
    pub logo_url: Option<String>,
    pub accent_color: Option<String>,
    pub notification_prefs: serde_json::Value,
    pub updated_at: String,
}

/// Request to update store settings (PATCH -- all fields optional).
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateStoreSettingsRequest {
    pub default_chain_id: Option<i64>,
    pub default_display_currency: Option<String>,
    pub logo_url: Option<String>,
    pub accent_color: Option<String>,
    pub notification_prefs: Option<serde_json::Value>,
}

/// Known webhook event types for notification_prefs validation.
pub(crate) const VALID_NOTIFICATION_EVENTS: &[&str] = &[
    "payment_detected",
    "payment_confirmed",
    "invoice_expired",
    "invoice_cancelled",
    "late_paid",
];

/// Get store settings.
#[utoipa::path(
    get,
    path = "/stores/{store_id}/settings",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID")
    ),
    responses(
        (status = 200, description = "Store settings", body = StoreSettingsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Store not found"),
    )
)]
pub async fn get_store_settings<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
) -> Result<Json<StoreSettingsResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    require_store_settings_permission(&state, &user, store_id).await?;

    // Verify store exists
    let _ = state
        .data_service
        .get_store(StoreId(store_id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let settings =
        data_service::StoreSettingsReader::get_store_settings(&*state.data_service, store_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    match settings {
        Some(s) => Ok(Json(StoreSettingsResponse {
            store_id: s.store_id,
            default_chain_id: s.default_chain_id,
            default_display_currency: s.default_display_currency,
            logo_url: s.logo_url,
            accent_color: s.accent_color,
            notification_prefs: s.notification_prefs,
            updated_at: s.updated_at.to_rfc3339(),
        })),
        None => Ok(Json(StoreSettingsResponse {
            store_id,
            default_chain_id: None,
            default_display_currency: None,
            logo_url: None,
            accent_color: None,
            notification_prefs: serde_json::json!({}),
            updated_at: chrono::Utc::now().to_rfc3339(),
        })),
    }
}

/// Update store settings (partial update).
#[utoipa::path(
    patch,
    path = "/stores/{store_id}/settings",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID")
    ),
    request_body = UpdateStoreSettingsRequest,
    responses(
        (status = 200, description = "Settings updated", body = StoreSettingsResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Store not found"),
    )
)]
#[allow(clippy::too_many_lines)] // PATCH handler validates + persists many optional fields
pub async fn update_store_settings<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
    Json(req): Json<UpdateStoreSettingsRequest>,
) -> Result<Json<StoreSettingsResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    require_store_settings_permission(&state, &user, store_id).await?;

    // Verify store exists
    let _ = state
        .data_service
        .get_store(StoreId(store_id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Validate default_chain_id
    if let Some(chain_id) = req.default_chain_id
        && evm::get_any_chain_config(chain_id as u64).is_none()
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Validate default_display_currency (ISO 4217 3-letter code)
    if let Some(ref currency) = req.default_display_currency
        && (currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_uppercase()))
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Validate logo_url (must be https://)
    if let Some(ref url) = req.logo_url
        && !url.starts_with("https://")
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Validate accent_color (must be #RRGGBB)
    if let Some(ref color) = req.accent_color
        && (color.len() != 7
            || !color.starts_with('#')
            || !color[1..].chars().all(|c| c.is_ascii_hexdigit()))
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Validate notification_prefs keys
    if let Some(ref prefs) = req.notification_prefs {
        if let Some(obj) = prefs.as_object() {
            for key in obj.keys() {
                if !VALID_NOTIFICATION_EVENTS.contains(&key.as_str()) {
                    return Err(StatusCode::BAD_REQUEST);
                }
            }
        } else {
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    // Merge with existing settings for partial update
    let existing =
        data_service::StoreSettingsReader::get_store_settings(&*state.data_service, store_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let empty_prefs = serde_json::json!({});
    let (chain_id, display_currency, logo, color, prefs) = match existing {
        Some(ref e) => (
            req.default_chain_id.or(e.default_chain_id),
            req.default_display_currency
                .as_deref()
                .or(e.default_display_currency.as_deref()),
            req.logo_url.as_deref().or(e.logo_url.as_deref()),
            req.accent_color.as_deref().or(e.accent_color.as_deref()),
            req.notification_prefs
                .as_ref()
                .unwrap_or(&e.notification_prefs),
        ),
        None => (
            req.default_chain_id,
            req.default_display_currency.as_deref(),
            req.logo_url.as_deref(),
            req.accent_color.as_deref(),
            req.notification_prefs.as_ref().unwrap_or(&empty_prefs),
        ),
    };

    let settings = data_service::StoreSettingsWriter::upsert_store_settings(
        &*state.data_service,
        store_id,
        chain_id,
        display_currency,
        logo,
        color,
        prefs,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(StoreSettingsResponse {
        store_id: settings.store_id,
        default_chain_id: settings.default_chain_id,
        default_display_currency: settings.default_display_currency,
        logo_url: settings.logo_url,
        accent_color: settings.accent_color,
        notification_prefs: settings.notification_prefs,
        updated_at: settings.updated_at.to_rfc3339(),
    }))
}
