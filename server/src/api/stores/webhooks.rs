//! Store webhook management endpoints: get, configure, delete.

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
use types::{StoreWebhookReader, StoreWebhookWriter};

use super::super::extractors::AuthenticatedUser;
use super::require_store_settings_permission;
use crate::state::PgAppState;

/// Request to configure a webhook.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ConfigureWebhookRequest {
    /// Webhook URL to receive notifications.
    pub webhook_url: String,
    /// Whether the webhook is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

/// Webhook response.
#[derive(Debug, Serialize, ToSchema)]
pub struct WebhookResponse {
    /// Webhook ID.
    pub id: Uuid,
    /// Store ID.
    pub store_id: Uuid,
    /// Webhook URL.
    pub webhook_url: String,
    /// Webhook secret (for signature verification).
    /// Only shown once when created/updated.
    pub webhook_secret: Option<String>,
    /// Whether the webhook is enabled.
    pub enabled: bool,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Last update timestamp.
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Get webhook configuration for a store.
#[utoipa::path(
    get,
    path = "/stores/{store_id}/webhook",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID")
    ),
    responses(
        (status = 200, description = "Webhook configuration", body = WebhookResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Webhook not configured"),
    )
)]
pub async fn get_store_webhook<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
) -> Result<Json<WebhookResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    require_store_settings_permission(&state, &user, store_id).await?;

    let webhook = StoreWebhookReader::get_webhook(&*state.data_service, store_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(WebhookResponse {
        id: webhook.id,
        store_id: webhook.store_id,
        webhook_url: webhook.webhook_url,
        webhook_secret: None, // Don't expose secret on GET
        enabled: webhook.enabled,
        created_at: webhook.created_at,
        updated_at: webhook.updated_at,
    }))
}

/// Configure webhook for a store.
///
/// Creates or updates the webhook configuration. A new secret is generated
/// on each update and returned in the response.
#[utoipa::path(
    put,
    path = "/stores/{store_id}/webhook",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID")
    ),
    request_body = ConfigureWebhookRequest,
    responses(
        (status = 200, description = "Webhook configured", body = WebhookResponse),
        (status = 400, description = "Invalid webhook URL"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Store not found"),
    )
)]
pub async fn configure_store_webhook<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
    Json(req): Json<ConfigureWebhookRequest>,
) -> Result<Json<WebhookResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    require_store_settings_permission(&state, &user, store_id).await?;

    // Validate webhook URL - must be HTTPS (or localhost for dev)
    if !req.webhook_url.starts_with("https://") && !req.webhook_url.starts_with("http://localhost")
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Verify store exists
    let _ = state
        .data_service
        .get_store(StoreId(store_id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Generate new secret
    let secret = uuid::Uuid::new_v4().to_string();

    let webhook = StoreWebhookWriter::upsert_webhook(
        &*state.data_service,
        store_id,
        &req.webhook_url,
        &secret,
        req.enabled,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(WebhookResponse {
        id: webhook.id,
        store_id: webhook.store_id,
        webhook_url: webhook.webhook_url,
        webhook_secret: Some(webhook.webhook_secret), // Show secret on create/update
        enabled: webhook.enabled,
        created_at: webhook.created_at,
        updated_at: webhook.updated_at,
    }))
}

/// Delete webhook configuration for a store.
#[utoipa::path(
    delete,
    path = "/stores/{store_id}/webhook",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID")
    ),
    responses(
        (status = 204, description = "Webhook deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Webhook not found"),
    )
)]
pub async fn delete_store_webhook<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
) -> Result<StatusCode, StatusCode>
where
    A: SessionService + 'static,
{
    require_store_settings_permission(&state, &user, store_id).await?;

    let deleted = StoreWebhookWriter::delete_webhook(&*state.data_service, store_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !deleted {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(StatusCode::NO_CONTENT)
}
