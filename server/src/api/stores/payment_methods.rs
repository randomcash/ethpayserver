//! Store payment method CRUD endpoints: list, create, get, update, delete.

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
use data_service::{self, StorePaymentMethod, StorePaymentMethodReader, StorePaymentMethodWriter};
use evm::validate_xpub;

use super::super::extractors::AuthenticatedUser;
use super::{mask_xpub, require_store_settings_permission};
use crate::state::PgAppState;

/// Request to create a payment method.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePaymentMethodRequest {
    /// Chain ID (e.g., 1 for Ethereum, 137 for Polygon, 11155111 for Sepolia).
    pub chain_id: u64,
    /// Token address for ERC20 tokens, null for native asset.
    pub token_address: Option<String>,
    /// Asset symbol (e.g., ETH, USDC).
    pub asset_symbol: String,
    /// Number of decimals for this asset (18 for ETH, 6 for USDC/USDT).
    pub decimals: u8,
    /// Extended public key for address derivation.
    pub xpub: String,
}

/// Request to update a payment method.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePaymentMethodRequest {
    /// Enable or disable the payment method.
    pub enabled: Option<bool>,
    /// Update the xpub.
    pub xpub: Option<String>,
}

/// Payment method response.
#[derive(Debug, Serialize, ToSchema)]
pub struct PaymentMethodResponse {
    /// Payment method ID.
    pub id: Uuid,
    /// Store ID.
    pub store_id: Uuid,
    /// Chain ID.
    pub chain_id: u64,
    /// Token address (null for native asset).
    pub token_address: Option<String>,
    /// Asset symbol.
    pub asset_symbol: String,
    /// Extended public key (masked).
    pub xpub_masked: String,
    /// Current derivation index.
    pub derivation_index: i32,
    /// Whether the payment method is enabled.
    pub enabled: bool,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<StorePaymentMethod> for PaymentMethodResponse {
    fn from(pm: StorePaymentMethod) -> Self {
        Self {
            id: pm.id,
            store_id: pm.store_id,
            chain_id: pm.chain_id,
            token_address: pm.token_address,
            asset_symbol: pm.asset_symbol,
            xpub_masked: mask_xpub(&pm.xpub),
            derivation_index: pm.derivation_index,
            enabled: pm.enabled,
            created_at: pm.created_at,
        }
    }
}

/// List payment methods for a store.
#[utoipa::path(
    get,
    path = "/stores/{store_id}/payment-methods",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID")
    ),
    responses(
        (status = 200, description = "List of payment methods", body = Vec<PaymentMethodResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
pub async fn list_payment_methods<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
) -> Result<Json<Vec<PaymentMethodResponse>>, StatusCode>
where
    A: SessionService + 'static,
{
    require_store_settings_permission(&state, &user, store_id).await?;

    let methods = StorePaymentMethodReader::get_payment_methods(&*state.data_service, store_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(methods.into_iter().map(|m| m.into()).collect()))
}

/// Create a payment method for a store.
#[utoipa::path(
    post,
    path = "/stores/{store_id}/payment-methods",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID")
    ),
    request_body = CreatePaymentMethodRequest,
    responses(
        (status = 201, description = "Payment method created", body = PaymentMethodResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Store not found"),
    )
)]
pub async fn create_payment_method<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
    Json(req): Json<CreatePaymentMethodRequest>,
) -> Result<(StatusCode, Json<PaymentMethodResponse>), StatusCode>
where
    A: SessionService + 'static,
{
    require_store_settings_permission(&state, &user, store_id).await?;

    // Validate xpub
    if !validate_xpub(&req.xpub) {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Verify store exists
    let _ = state
        .data_service
        .get_store(StoreId(store_id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let method = StorePaymentMethodWriter::create_payment_method(
        &*state.data_service,
        store_id,
        req.chain_id,
        req.token_address.as_deref(),
        &req.asset_symbol,
        req.decimals,
        &req.xpub,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok((StatusCode::CREATED, Json(method.into())))
}

/// Get a specific payment method.
#[utoipa::path(
    get,
    path = "/stores/{store_id}/payment-methods/{method_id}",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID"),
        ("method_id" = Uuid, Path, description = "Payment method ID")
    ),
    responses(
        (status = 200, description = "Payment method details", body = PaymentMethodResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Payment method not found"),
    )
)]
pub async fn get_payment_method<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path((store_id, method_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<PaymentMethodResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    require_store_settings_permission(&state, &user, store_id).await?;

    let method = StorePaymentMethodReader::get_payment_method(&*state.data_service, method_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Verify it belongs to this store
    if method.store_id != store_id {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(Json(method.into()))
}

/// Update a payment method.
#[utoipa::path(
    put,
    path = "/stores/{store_id}/payment-methods/{method_id}",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID"),
        ("method_id" = Uuid, Path, description = "Payment method ID")
    ),
    request_body = UpdatePaymentMethodRequest,
    responses(
        (status = 200, description = "Payment method updated", body = PaymentMethodResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Payment method not found"),
    )
)]
pub async fn update_payment_method<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path((store_id, method_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdatePaymentMethodRequest>,
) -> Result<Json<PaymentMethodResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    require_store_settings_permission(&state, &user, store_id).await?;

    // Validate xpub if provided
    if let Some(ref xpub) = req.xpub
        && !validate_xpub(xpub)
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Verify method exists and belongs to this store
    let existing = StorePaymentMethodReader::get_payment_method(&*state.data_service, method_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if existing.store_id != store_id {
        return Err(StatusCode::NOT_FOUND);
    }

    let method = StorePaymentMethodWriter::update_payment_method(
        &*state.data_service,
        method_id,
        req.enabled,
        req.xpub.as_deref(),
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(method.into()))
}

/// Delete a payment method.
#[utoipa::path(
    delete,
    path = "/stores/{store_id}/payment-methods/{method_id}",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID"),
        ("method_id" = Uuid, Path, description = "Payment method ID")
    ),
    responses(
        (status = 204, description = "Payment method deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Payment method not found"),
    )
)]
pub async fn delete_payment_method<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path((store_id, method_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, StatusCode>
where
    A: SessionService + 'static,
{
    require_store_settings_permission(&state, &user, store_id).await?;

    // Verify method exists and belongs to this store
    let existing = StorePaymentMethodReader::get_payment_method(&*state.data_service, method_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if existing.store_id != store_id {
        return Err(StatusCode::NOT_FOUND);
    }

    StorePaymentMethodWriter::delete_payment_method(&*state.data_service, method_id)
        .await
        .map_err(|e| match e {
            data_service::RepositoryError::NotFound(_) => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        })?;

    Ok(StatusCode::NO_CONTENT)
}
