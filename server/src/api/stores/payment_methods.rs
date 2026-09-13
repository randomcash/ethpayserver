//! Store payment method CRUD endpoints: list, create, get, update, delete.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use uuid::Uuid;

use ::types::ChainId;
use auth::repository::StoreRepository;
use auth::{ServerSettings, ServerSettingsRepository, SessionService, StoreId};
use data_service::{self, StorePaymentMethodReader, StorePaymentMethodWriter};
use evm::validate_xpub;

use super::super::extractors::AuthenticatedUser;
use super::{ApiErr, repository_error, require_store_settings_permission};
use crate::state::PgAppState;
pub use api_types::{
    CreatePaymentMethodRequest, PaymentMethodResponse, UpdatePaymentMethodRequest,
};

/// Whether the server has a registered adapter for this chain.
///
/// When a settings row exists, this is membership in
/// `enabled_chain_ids` - the operator's list of chains something on this
/// server actually watches - not "is this an EVM chain", because that would
/// hardcode today's only adapter into the check. A Tron adapter registers by
/// the operator adding `tron:...` to that list; this predicate does not
/// change.
///
/// `settings` is `None` when nobody has ever written a `server_settings`
/// row - confirmed true of the live testnet database as of this change
/// (`SELECT * FROM server_settings` returns zero rows there), and unverified
/// but plausibly also true of mainnet. Falling back to
/// `ServerSettings::default()` in that case, as an earlier version of this
/// check did, silently turns "reject Tron" into "reject every chain this
/// deployment actually serves": that default is a Rust-side, EVM-mainnet
/// chain list baked in at compile time, and testnet's only real chain
/// (`eip155:11155111`, Sepolia) is not on it - the "unchanged 201" half of
/// this ticket's own required tests would have failed against the databases
/// this server actually runs on. Until an operator populates a real row,
/// `is_evm()` is the honest fallback: it is, today, an exact description of
/// "has a registered adapter" (the only adapter that exists is the EVM one),
/// and it costs nothing once a real Tron row lands, because a configured
/// `Some` always wins.
///
/// Without this gate at all a merchant can register e.g. `tron:728126428`,
/// which still derives a secp256k1 address (the same curve as EVM) but that
/// address is never watched - the invoice can be paid and is never marked so.
pub(crate) fn chain_has_no_adapter(chain_id: &ChainId, settings: Option<&ServerSettings>) -> bool {
    match settings {
        Some(settings) => !settings.enabled_chain_ids.contains(chain_id),
        None => !chain_id.is_evm(),
    }
}

/// Whether an update to a payment method should still be checked against
/// `chain_has_no_adapter`.
///
/// The check only ever fires for a legacy row that predates this ticket -
/// `UpdatePaymentMethodRequest` has no `chain_id`, so a fresh row can never
/// fail it. If it also blocked `enabled: false` on such a row, the only way
/// to turn one off would be direct database surgery, which is exactly what
/// the ticket's audit step is for someone to be able to avoid. Disabling
/// only reduces exposure, so it is let through unconditionally; anything
/// else on a bad row (re-enabling it, rotating its xpub) is still refused.
pub(crate) fn update_should_check_chain(requested_enabled: Option<bool>) -> bool {
    requested_enabled != Some(false)
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

    // A key is optional now: omitted means "use the one this store already
    // resolves to", so a merchant pastes it once rather than per chain, per
    // token and per store. When one IS given it still has to be a real
    // extended PUBLIC key - `validate_xpub` refuses an `xprv` on the version
    // byte, which is what keeps this non-custodial even if someone pastes the
    // wrong line out of their wallet.
    if let Some(ref xpub) = req.xpub
        && !validate_xpub(xpub)
    {
        return Err(StatusCode::BAD_REQUEST.into());
    }

    // Refuse a chain nothing here can watch. See `chain_has_no_adapter` -
    // otherwise the option gets an address (the same curve derives one for
    // any namespace) and quotes a customer who can pay it while nothing
    // notices the money arrived.
    let settings = state
        .data_service
        .get_server_settings()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if chain_has_no_adapter(&req.chain_id, settings.as_ref()) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "unsupported_chain: no adapter is registered for {}",
                req.chain_id
            ),
        )
            .into());
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
        req.xpub.as_deref(),
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

    // Same gate as creation, against the chain already stored on this method -
    // `UpdatePaymentMethodRequest` carries no chain_id of its own, so there is
    // nothing to validate on the request. This only bites a row from before
    // this check existed (see the RCS-281 commit message for the audit); a
    // fresh row can never have an unsupported chain.
    if update_should_check_chain(req.enabled) {
        let settings = state
            .data_service
            .get_server_settings()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if chain_has_no_adapter(&existing.chain_id, settings.as_ref()) {
            return Err((
                StatusCode::BAD_REQUEST,
                format!(
                    "unsupported_chain: no adapter is registered for {}",
                    existing.chain_id
                ),
            )
                .into());
        }
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
