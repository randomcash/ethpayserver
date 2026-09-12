//! Store settings endpoints: get and update (PATCH).

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use uuid::Uuid;

use auth::repository::StoreRepository;
use auth::{SessionService, StoreId};

use super::super::extractors::AuthenticatedUser;
use super::require_store_settings_permission;
use crate::state::PgAppState;
pub use api_types::{StoreSettingsResponse, UpdateStoreSettingsRequest};

/// Known webhook event types for notification_prefs validation.
///
/// These carry an object value (`{"webhook": true}`) describing the channels
/// for that event.
/// Kept in step with `WebhookEventType` by
/// `test_every_webhook_event_is_configurable`: an event missing from this list
/// cannot be switched off, and a name here that no event uses is a switch
/// wired to nothing.
pub(crate) const VALID_NOTIFICATION_EVENTS: &[&str] = &[
    "payment_detected",
    "payment_confirmed",
    "payment_reorged",
    "invoice_expired",
    "invoice_cancelled",
    "late_paid",
];

/// Channel switches that share the `notification_prefs` blob with the events
/// above but are plain booleans, not per-event channel maps.
///
/// `customer_receipts_enabled` used to be missing from validation entirely, so
/// `PUT /stores/{id}/settings` answered 400 to any payload carrying it. A
/// client could therefore only save notification preferences by dropping the
/// key - and since the update replaced the blob wholesale, dropping it turned
/// customer receipt emails back ON for a merchant who had switched them off.
/// `receipts_disabled_for_store` treats absent as enabled.
pub(crate) const VALID_NOTIFICATION_SWITCHES: &[&str] = &["customer_receipts_enabled"];

/// Check every key and value in a `notification_prefs` payload.
///
/// The blob holds two shapes: per-event channel maps (`{"webhook": true}`) and
/// plain boolean switches. Validation used to know only about the events, so
/// any payload carrying `customer_receipts_enabled` was a 400 - see
/// [`VALID_NOTIFICATION_SWITCHES`].
///
/// Values are checked too, not just keys: a switch sent as the string `"false"`
/// would store cleanly and then read as enabled, because
/// `receipts_disabled_for_store` matches on `Bool(false)` exactly. `null` is
/// accepted for either shape and means "remove this key" once merged.
pub(crate) fn validate_notification_prefs(prefs: &serde_json::Value) -> Result<(), StatusCode> {
    let Some(obj) = prefs.as_object() else {
        return Err(StatusCode::BAD_REQUEST);
    };
    for (key, value) in obj {
        let ok = if VALID_NOTIFICATION_EVENTS.contains(&key.as_str()) {
            value.is_object() || value.is_null()
        } else if VALID_NOTIFICATION_SWITCHES.contains(&key.as_str()) {
            value.is_boolean() || value.is_null()
        } else {
            false
        };
        if !ok {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    Ok(())
}

/// Merge an incoming `notification_prefs` patch over what is stored.
///
/// Top-level keys only, which matches the blob's shape: events map to a channel
/// object, switches to a boolean. An omitted key keeps its stored value; an
/// explicit `null` removes it.
///
/// This replaces a wholesale swap of the blob. Under that, saving any single
/// preference dropped every key the client had not sent - and a dropped
/// `customer_receipts_enabled` reads as enabled, so a merchant who had turned
/// customer emails off started sending them again by saving something else.
/// Allowing the key through validation fixes that one instance; merging is what
/// stops the next field from repeating it.
pub(crate) fn merge_notification_prefs(
    stored: &serde_json::Value,
    patch: &serde_json::Value,
) -> serde_json::Value {
    let mut out = stored.as_object().cloned().unwrap_or_default();
    if let Some(patch) = patch.as_object() {
        for (key, value) in patch {
            if value.is_null() {
                out.remove(key);
            } else {
                out.insert(key.clone(), value.clone());
            }
        }
    }
    serde_json::Value::Object(out)
}

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
        (status = 422, description = "Malformed body — e.g. a chain id that is not CAIP-2"),
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

    // Validate default_chain_id. This server only serves EVM chains, so a
    // well-formed identifier from another family is still not one it can quote.
    if let Some(ref chain_id) = req.default_chain_id
        && chain_id
            .evm_chain_id()
            .and_then(evm::get_any_chain_config)
            .is_none()
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

    if let Some(ref prefs) = req.notification_prefs {
        validate_notification_prefs(prefs)?;
    }

    // Merge with existing settings for partial update
    let existing =
        data_service::StoreSettingsReader::get_store_settings(&*state.data_service, store_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let empty_prefs = serde_json::json!({});
    let (chain_id, display_currency, logo, color, prefs) = match existing {
        Some(ref e) => (
            req.default_chain_id.clone().or(e.default_chain_id.clone()),
            req.default_display_currency
                .as_deref()
                .or(e.default_display_currency.as_deref()),
            req.logo_url.as_deref().or(e.logo_url.as_deref()),
            req.accent_color.as_deref().or(e.accent_color.as_deref()),
            req.notification_prefs.as_ref().map_or_else(
                || e.notification_prefs.clone(),
                |patch| merge_notification_prefs(&e.notification_prefs, patch),
            ),
        ),
        None => (
            req.default_chain_id,
            req.default_display_currency.as_deref(),
            req.logo_url.as_deref(),
            req.accent_color.as_deref(),
            req.notification_prefs.as_ref().map_or_else(
                || empty_prefs.clone(),
                |patch| merge_notification_prefs(&empty_prefs, patch),
            ),
        ),
    };

    let settings = data_service::StoreSettingsWriter::upsert_store_settings(
        &*state.data_service,
        store_id,
        chain_id.as_ref(),
        display_currency,
        logo,
        color,
        &prefs,
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
