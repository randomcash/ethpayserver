//! Store payment method CRUD endpoints: list, create, get, update, delete.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use uuid::Uuid;

use auth::repository::StoreRepository;
use auth::{SessionService, StoreId};
use data_service::{self, StorePaymentMethodReader, StorePaymentMethodWriter};
use evm::validate_xpub;

use super::super::extractors::AuthenticatedUser;
use super::{ApiErr, repository_error, require_store_settings_permission};
use crate::state::PgAppState;
pub use api_types::{
    CreatePaymentMethodRequest, PaymentMethodResponse, UpdatePaymentMethodRequest,
};

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
        (status = 409, description = "That xpub is registered to another account"),
        (status = 400, description = "Invalid request"),
        (status = 422, description = "Malformed body — e.g. a chain id that is not CAIP-2"),
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
) -> Result<(StatusCode, Json<PaymentMethodResponse>), ApiErr>
where
    A: SessionService + 'static,
{
    require_store_settings_permission(&state, &user, store_id).await?;

    // Validate xpub
    if !validate_xpub(&req.xpub) {
        return Err(StatusCode::BAD_REQUEST.into());
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
        &req.chain_id,
        req.token_address.as_deref(),
        &req.asset_symbol,
        req.decimals,
        &req.xpub,
    )
    .await
    .map_err(repository_error)?;

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
        (status = 409, description = "That xpub is registered to another account"),
        (status = 400, description = "Invalid request"),
        (status = 422, description = "Malformed body — e.g. a chain id that is not CAIP-2"),
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
) -> Result<Json<PaymentMethodResponse>, ApiErr>
where
    A: SessionService + 'static,
{
    require_store_settings_permission(&state, &user, store_id).await?;

    // Validate xpub if provided
    if let Some(ref xpub) = req.xpub
        && !validate_xpub(xpub)
    {
        return Err(StatusCode::BAD_REQUEST.into());
    }

    // Verify method exists and belongs to this store
    let existing = StorePaymentMethodReader::get_payment_method(&*state.data_service, method_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if existing.store_id != store_id {
        return Err(StatusCode::NOT_FOUND.into());
    }

    let method = StorePaymentMethodWriter::update_payment_method(
        &*state.data_service,
        method_id,
        req.enabled,
        req.xpub.as_deref(),
    )
    .await
    .map_err(repository_error)?;

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
