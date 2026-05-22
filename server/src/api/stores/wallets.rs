//! Store wallet configuration endpoints: list, get, configure, delete, xpub export, addresses, rotation.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use auth::repository::{StoreRepository, UserStoreRepository};
use auth::{SessionService, StoreId};
use data_service::{self, StorePaymentMethodReader, StoreWalletReader, StoreWalletWriter};
use evm::{XpubDeriver, validate_xpub};

use super::super::extractors::AuthenticatedUser;
use super::{mask_xpub, require_store_settings_permission};
use crate::state::PgAppState;

/// Request to configure a store wallet.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ConfigureWalletRequest {
    /// Extended public key (xpub) for address derivation.
    pub xpub: String,
    /// Optional wallet name.
    pub name: Option<String>,
}

/// Store wallet response.
#[derive(Debug, Serialize, ToSchema)]
pub struct WalletResponse {
    /// Wallet ID.
    pub id: Uuid,
    /// Store ID.
    pub store_id: Uuid,
    /// Extended public key (masked for security).
    pub xpub_masked: String,
    /// Current derivation index.
    pub derivation_index: i32,
    /// Wallet name.
    pub name: Option<String>,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Wallet xpub export response (full, unmasked).
#[derive(Debug, Serialize, ToSchema)]
pub struct WalletXpubResponse {
    /// Wallet ID.
    pub id: Uuid,
    /// Store ID.
    pub store_id: Uuid,
    /// Full extended public key (unmasked).
    pub xpub: String,
    /// Current derivation index.
    pub derivation_index: i32,
    /// Wallet name.
    pub name: Option<String>,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// A derived wallet address with its index and derivation path.
#[derive(Debug, Serialize, ToSchema)]
pub struct DerivedAddressEntry {
    /// Ethereum address (checksummed hex).
    pub address: String,
    /// BIP-44 derivation index.
    pub index: u32,
    /// Full BIP-44 derivation path.
    pub derivation_path: String,
    /// Whether this index has been assigned to a payment option.
    pub used: bool,
}

/// Response for listing derived wallet addresses.
#[derive(Debug, Serialize, ToSchema)]
pub struct WalletAddressesResponse {
    /// Wallet ID.
    pub wallet_id: Uuid,
    /// Current derivation index (number of addresses assigned so far).
    pub derivation_index: i32,
    /// Derived addresses.
    pub addresses: Vec<DerivedAddressEntry>,
}

/// Query parameters for listing wallet addresses.
#[derive(Debug, Deserialize, IntoParams)]
pub struct WalletAddressesQuery {
    /// Number of addresses to derive (default 20, max 100).
    pub limit: Option<u32>,
    /// Starting index (default 0).
    pub offset: Option<u32>,
}

/// List all wallets for stores the user is a member of.
#[utoipa::path(
    get,
    path = "/wallets",
    tag = "stores",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "List of user's wallets", body = Vec<WalletResponse>),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn list_wallets<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
) -> Result<Json<Vec<WalletResponse>>, StatusCode>
where
    A: SessionService + 'static,
{
    // Get user's stores
    let stores = state
        .data_service
        .get_stores_for_user(user.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut wallets = Vec::new();

    // Fetch wallet for each store
    for store in stores {
        if let Ok(Some(wallet)) =
            StoreWalletReader::get_wallet(&*state.data_service, store.id.0).await
        {
            wallets.push(WalletResponse {
                id: wallet.id,
                store_id: wallet.store_id,
                xpub_masked: mask_xpub(&wallet.xpub),
                derivation_index: wallet.derivation_index,
                name: wallet.name,
                created_at: wallet.created_at,
            });
        }
    }

    Ok(Json(wallets))
}

/// Get a wallet by ID.
///
/// User must be a member of the wallet's store.
#[utoipa::path(
    get,
    path = "/wallets/{wallet_id}",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("wallet_id" = Uuid, Path, description = "Wallet ID")
    ),
    responses(
        (status = 200, description = "Wallet details", body = WalletResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Not a member of this wallet's store"),
        (status = 404, description = "Wallet not found"),
    )
)]
pub async fn get_wallet_by_id<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(wallet_id): Path<Uuid>,
) -> Result<Json<WalletResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let wallet = StoreWalletReader::get_wallet_by_id(&*state.data_service, wallet_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Verify user has access to the wallet's store
    let has_permission = state
        .data_service
        .user_has_store_permission(
            user.id,
            StoreId(wallet.store_id),
            "ethpay.store.canviewstoresettings",
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !has_permission {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(Json(WalletResponse {
        id: wallet.id,
        store_id: wallet.store_id,
        xpub_masked: mask_xpub(&wallet.xpub),
        derivation_index: wallet.derivation_index,
        name: wallet.name,
        created_at: wallet.created_at,
    }))
}

/// Export the full (unmasked) xpub for a wallet.
///
/// Requires `canviewstoresettings` permission on the wallet's store.
#[utoipa::path(
    get,
    path = "/wallets/{wallet_id}/xpub",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("wallet_id" = Uuid, Path, description = "Wallet ID")
    ),
    responses(
        (status = 200, description = "Full xpub export", body = WalletXpubResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Not a member of this wallet's store"),
        (status = 404, description = "Wallet not found"),
    )
)]
pub async fn export_wallet_xpub<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(wallet_id): Path<Uuid>,
) -> Result<Json<WalletXpubResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let wallet = StoreWalletReader::get_wallet_by_id(&*state.data_service, wallet_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let has_permission = state
        .data_service
        .user_has_store_permission(
            user.id,
            StoreId(wallet.store_id),
            "ethpay.store.canviewstoresettings",
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !has_permission {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(Json(WalletXpubResponse {
        id: wallet.id,
        store_id: wallet.store_id,
        xpub: wallet.xpub,
        derivation_index: wallet.derivation_index,
        name: wallet.name,
        created_at: wallet.created_at,
    }))
}

/// List derived addresses for a wallet.
///
/// Derives addresses from the wallet's xpub at the requested index range.
/// Addresses below the current `derivation_index` are marked as used.
#[utoipa::path(
    get,
    path = "/wallets/{wallet_id}/addresses",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("wallet_id" = Uuid, Path, description = "Wallet ID"),
        WalletAddressesQuery,
    ),
    responses(
        (status = 200, description = "Derived addresses", body = WalletAddressesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Not a member of this wallet's store"),
        (status = 404, description = "Wallet not found"),
        (status = 500, description = "Address derivation failed"),
    )
)]
pub async fn list_wallet_addresses<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(wallet_id): Path<Uuid>,
    Query(query): Query<WalletAddressesQuery>,
) -> Result<Json<WalletAddressesResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let wallet = StoreWalletReader::get_wallet_by_id(&*state.data_service, wallet_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let has_permission = state
        .data_service
        .user_has_store_permission(
            user.id,
            StoreId(wallet.store_id),
            "ethpay.store.canviewstoresettings",
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !has_permission {
        return Err(StatusCode::FORBIDDEN);
    }

    let limit = query.limit.unwrap_or(20).min(100);
    let offset = query.offset.unwrap_or(0);

    let deriver =
        XpubDeriver::from_xpub(&wallet.xpub).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut addresses = Vec::with_capacity(limit as usize);
    for i in offset..offset + limit {
        let address = deriver
            .derive_address(i)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        addresses.push(DerivedAddressEntry {
            address: address.to_string(),
            index: i,
            derivation_path: format!("m/44'/60'/0'/0/{}", i),
            used: (i as i32) < wallet.derivation_index,
        });
    }

    Ok(Json(WalletAddressesResponse {
        wallet_id: wallet.id,
        derivation_index: wallet.derivation_index,
        addresses,
    }))
}

/// Get wallet configuration for a store.
#[utoipa::path(
    get,
    path = "/stores/{store_id}/wallet",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID")
    ),
    responses(
        (status = 200, description = "Wallet configuration", body = WalletResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Store or wallet not found"),
    )
)]
pub async fn get_store_wallet<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
) -> Result<Json<WalletResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    // Check permission
    let has_permission = state
        .data_service
        .user_has_store_permission(
            user.id,
            StoreId(store_id),
            "ethpay.store.canviewstoresettings",
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !has_permission {
        return Err(StatusCode::FORBIDDEN);
    }

    let wallet = StoreWalletReader::get_wallet(&*state.data_service, store_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(WalletResponse {
        id: wallet.id,
        store_id: wallet.store_id,
        xpub_masked: mask_xpub(&wallet.xpub),
        derivation_index: wallet.derivation_index,
        name: wallet.name,
        created_at: wallet.created_at,
    }))
}

/// Configure wallet for a store.
///
/// Provide an extended public key (xpub) to enable payment address derivation.
#[utoipa::path(
    put,
    path = "/stores/{store_id}/wallet",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID")
    ),
    request_body = ConfigureWalletRequest,
    responses(
        (status = 200, description = "Wallet configured", body = WalletResponse),
        (status = 400, description = "Invalid xpub"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Store not found"),
    )
)]
pub async fn configure_store_wallet<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
    Json(req): Json<ConfigureWalletRequest>,
) -> Result<Json<WalletResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    // Check permission
    let has_permission = state
        .data_service
        .user_has_store_permission(
            user.id,
            StoreId(store_id),
            "ethpay.store.canmodifystoresettings",
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !has_permission {
        return Err(StatusCode::FORBIDDEN);
    }

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

    let wallet = StoreWalletWriter::upsert_wallet(
        &*state.data_service,
        store_id,
        &req.xpub,
        req.name.as_deref(),
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(WalletResponse {
        id: wallet.id,
        store_id: wallet.store_id,
        xpub_masked: mask_xpub(&wallet.xpub),
        derivation_index: wallet.derivation_index,
        name: wallet.name,
        created_at: wallet.created_at,
    }))
}

/// Delete wallet configuration for a store.
#[utoipa::path(
    delete,
    path = "/stores/{store_id}/wallet",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID")
    ),
    responses(
        (status = 204, description = "Wallet deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Wallet not found"),
    )
)]
pub async fn delete_store_wallet<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
) -> Result<StatusCode, StatusCode>
where
    A: SessionService + 'static,
{
    // Check permission
    let has_permission = state
        .data_service
        .user_has_store_permission(
            user.id,
            StoreId(store_id),
            "ethpay.store.canmodifystoresettings",
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !has_permission {
        return Err(StatusCode::FORBIDDEN);
    }

    StoreWalletWriter::delete_wallet(&*state.data_service, store_id)
        .await
        .map_err(|e| match e {
            data_service::RepositoryError::NotFound(_) => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        })?;

    Ok(StatusCode::NO_CONTENT)
}

// =============================================================================
// Wallet XPub Rotation
// =============================================================================

/// Request to rotate wallet xpub.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RotateWalletRequest {
    /// New extended public key to rotate to.
    pub xpub: String,
    /// Optional reason for rotation (e.g., "key compromise", "scheduled rotation").
    pub reason: Option<String>,
}

/// A single rotation event in the response.
#[derive(Debug, Serialize, ToSchema)]
pub struct RotationEntry {
    /// Rotation ID.
    pub id: Uuid,
    /// Payment method that was rotated.
    pub payment_method_id: Uuid,
    /// Chain ID of the rotated payment method.
    pub chain_id: u64,
    /// Asset symbol of the rotated payment method.
    pub asset_symbol: String,
    /// Previous xpub (masked).
    pub previous_xpub_masked: String,
    /// Derivation index at time of rotation.
    pub previous_derivation_index: i32,
    /// When the rotation occurred.
    pub rotated_at: chrono::DateTime<chrono::Utc>,
}

/// Response from wallet rotation.
#[derive(Debug, Serialize, ToSchema)]
pub struct RotateWalletResponse {
    /// Store ID.
    pub store_id: Uuid,
    /// New xpub (masked).
    pub new_xpub_masked: String,
    /// Number of payment methods rotated.
    pub methods_rotated: usize,
    /// Individual rotation entries.
    pub rotations: Vec<RotationEntry>,
}

/// Rotate wallet xpub for a store.
///
/// Replaces the xpub on all payment methods for this store, resetting derivation
/// indices to zero. Old addresses remain watched until their parent invoices
/// resolve. A rotation history record is kept for each payment method.
#[utoipa::path(
    post,
    path = "/stores/{store_id}/wallet/rotate",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID")
    ),
    request_body = RotateWalletRequest,
    responses(
        (status = 200, description = "Wallet rotated", body = RotateWalletResponse),
        (status = 400, description = "Invalid xpub or same as current"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "No payment methods found for store"),
    )
)]
pub async fn rotate_store_wallet<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
    Json(req): Json<RotateWalletRequest>,
) -> Result<Json<RotateWalletResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    require_store_settings_permission(&state, &user, store_id).await?;

    // Validate the new xpub
    if !validate_xpub(&req.xpub) {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Verify store exists
    let _ = state
        .data_service
        .get_store(auth::StoreId(store_id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Get all payment methods for this store
    let methods = StorePaymentMethodReader::get_payment_methods(&*state.data_service, store_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if methods.is_empty() {
        return Err(StatusCode::NOT_FOUND);
    }

    // Reject if all methods already use this xpub (pointless rotation)
    if methods.iter().all(|m| m.xpub == req.xpub) {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Rotate each payment method that has a different xpub
    let mut rotations = Vec::new();
    for method in &methods {
        if method.xpub == req.xpub {
            continue;
        }

        let rotation = state
            .data_service
            .rotate_payment_method_xpub(store_id, method.id, &req.xpub, req.reason.as_deref())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        rotations.push(RotationEntry {
            id: rotation.id,
            payment_method_id: method.id,
            chain_id: method.chain_id,
            asset_symbol: method.asset_symbol.clone(),
            previous_xpub_masked: mask_xpub(&rotation.previous_xpub),
            previous_derivation_index: rotation.previous_derivation_index,
            rotated_at: rotation.rotated_at,
        });
    }

    Ok(Json(RotateWalletResponse {
        store_id,
        new_xpub_masked: mask_xpub(&req.xpub),
        methods_rotated: rotations.len(),
        rotations,
    }))
}
