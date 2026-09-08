//! Account wallet endpoints, and the per-store override that points at one.
//!
//! Wallets belong to the account, not to a store (RCS-234). A store derives
//! from its own override if it has been given one, and from the account
//! primary otherwise; the fallback is resolved in the repository so no handler
//! can spell it differently.

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
use data_service::{self, StorePaymentMethodReader, WalletReader, WalletWriter};
use evm::{XpubDeriver, validate_xpub};

use super::super::extractors::AuthenticatedUser;
use super::{mask_xpub, require_store_settings_permission};
use crate::state::PgAppState;

/// Request to add a wallet to the account.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateWalletRequest {
    /// Extended public key (xpub) for address derivation.
    pub xpub: String,
    /// Optional wallet name.
    pub name: Option<String>,
}

/// Request to update a wallet.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateWalletRequest {
    /// New name. Absent leaves the name alone.
    pub name: Option<String>,
    /// Set to `true` to make this the account primary. `false` is ignored:
    /// an account either has a primary or is choosing a different one, and
    /// "no primary" is not a state a merchant can usefully ask for.
    pub is_primary: Option<bool>,
}

/// Request to point a store at a wallet.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetStoreWalletRequest {
    /// Wallet to use for this store. Must belong to the same account.
    pub wallet_id: Uuid,
}

/// Account wallet response.
#[derive(Debug, Serialize, ToSchema)]
pub struct WalletResponse {
    /// Wallet ID.
    pub id: Uuid,
    /// Owning account.
    pub user_id: Uuid,
    /// Extended public key (masked for security).
    pub xpub_masked: String,
    /// Next derivation index this wallet will issue.
    pub derivation_index: i32,
    /// Wallet name.
    pub name: Option<String>,
    /// Whether stores fall back to this wallet.
    pub is_primary: bool,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<data_service::Wallet> for WalletResponse {
    fn from(w: data_service::Wallet) -> Self {
        Self {
            id: w.id,
            user_id: w.user_id,
            xpub_masked: mask_xpub(&w.xpub),
            derivation_index: w.derivation_index,
            name: w.name,
            is_primary: w.is_primary,
            created_at: w.created_at,
        }
    }
}

/// The wallet a store derives from, and how it got there.
#[derive(Debug, Serialize, ToSchema)]
pub struct StoreWalletResponse {
    /// Store ID.
    pub store_id: Uuid,
    /// The wallet this store's addresses come from.
    #[serde(flatten)]
    pub wallet: WalletResponse,
    /// True when the store is pinned to this wallet, false when it is simply
    /// following the account primary. The distinction is what tells a merchant
    /// whether changing their primary will move this store's payouts.
    pub is_override: bool,
}

/// Wallet xpub export response (full, unmasked).
#[derive(Debug, Serialize, ToSchema)]
pub struct WalletXpubResponse {
    /// Wallet ID.
    pub id: Uuid,
    /// Owning account.
    pub user_id: Uuid,
    /// Full extended public key (unmasked).
    pub xpub: String,
    /// Next derivation index this wallet will issue.
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
    /// Next derivation index (number of addresses assigned so far).
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

/// Load a wallet and confirm it belongs to the caller.
///
/// Ownership is the whole authorization story for a wallet - there is no
/// sharing - so a wallet on another account is reported as 404 rather than
/// 403: confirming it exists would let anyone probe for wallet ids.
async fn owned_wallet<A: SessionService>(
    state: &PgAppState<A>,
    user: &auth::UserInfo,
    wallet_id: Uuid,
) -> Result<data_service::Wallet, StatusCode> {
    let wallet = WalletReader::get_wallet(&*state.data_service, wallet_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if wallet.user_id != user.id.0 {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(wallet)
}

/// List the account's wallets.
#[utoipa::path(
    get,
    path = "/wallets",
    tag = "stores",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The account's wallets", body = Vec<WalletResponse>),
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
    let wallets = WalletReader::list_wallets(&*state.data_service, user.id.0)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(wallets.into_iter().map(Into::into).collect()))
}

/// Add a wallet to the account.
///
/// Adding an xpub the account already holds returns the existing wallet rather
/// than a duplicate: two rows on one key would be two derivation counters on
/// it, and that is exactly the collision the account-level model removes.
#[utoipa::path(
    post,
    path = "/wallets",
    tag = "stores",
    security(("bearer_auth" = [])),
    request_body = CreateWalletRequest,
    responses(
        (status = 201, description = "Wallet added", body = WalletResponse),
        (status = 400, description = "Invalid xpub"),
        (status = 401, description = "Unauthorized"),
        (status = 409, description = "Another account already holds this xpub"),
    )
)]
pub async fn create_wallet<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Json(req): Json<CreateWalletRequest>,
) -> Result<(StatusCode, Json<WalletResponse>), StatusCode>
where
    A: SessionService + 'static,
{
    if !validate_xpub(&req.xpub) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let wallet = WalletWriter::create_wallet(
        &*state.data_service,
        user.id.0,
        &req.xpub,
        req.name.as_deref(),
    )
    .await
    .map_err(|e| match e {
        data_service::RepositoryError::Conflict(_) => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    })?;

    Ok((StatusCode::CREATED, Json(wallet.into())))
}

/// Get one of the account's wallets.
#[utoipa::path(
    get,
    path = "/wallets/{wallet_id}",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(("wallet_id" = Uuid, Path, description = "Wallet ID")),
    responses(
        (status = 200, description = "Wallet details", body = WalletResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "No such wallet on this account"),
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
    Ok(Json(owned_wallet(&state, &user, wallet_id).await?.into()))
}

/// Rename a wallet, or make it the account primary.
#[utoipa::path(
    patch,
    path = "/wallets/{wallet_id}",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(("wallet_id" = Uuid, Path, description = "Wallet ID")),
    request_body = UpdateWalletRequest,
    responses(
        (status = 200, description = "Updated wallet", body = WalletResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "No such wallet on this account"),
    )
)]
pub async fn update_wallet<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(wallet_id): Path<Uuid>,
    Json(req): Json<UpdateWalletRequest>,
) -> Result<Json<WalletResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let mut wallet = owned_wallet(&state, &user, wallet_id).await?;

    if req.name.is_some() {
        wallet = WalletWriter::rename_wallet(&*state.data_service, wallet_id, req.name.as_deref())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    if req.is_primary == Some(true) {
        wallet = WalletWriter::set_primary_wallet(&*state.data_service, user.id.0, wallet_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    Ok(Json(wallet.into()))
}

/// Remove a wallet from the account.
///
/// Refused while a store or a payment method still points at it. The addresses
/// it derived are still being watched, and dropping the xpub would leave
/// incoming payments with no key to attribute them to.
#[utoipa::path(
    delete,
    path = "/wallets/{wallet_id}",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(("wallet_id" = Uuid, Path, description = "Wallet ID")),
    responses(
        (status = 204, description = "Wallet deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "No such wallet on this account"),
        (status = 409, description = "Still in use by a store or payment method"),
    )
)]
pub async fn delete_wallet<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(wallet_id): Path<Uuid>,
) -> Result<StatusCode, StatusCode>
where
    A: SessionService + 'static,
{
    owned_wallet(&state, &user, wallet_id).await?;

    WalletWriter::delete_wallet(&*state.data_service, wallet_id)
        .await
        .map_err(|e| match e {
            data_service::RepositoryError::Conflict(_) => StatusCode::CONFLICT,
            data_service::RepositoryError::NotFound(_) => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        })?;

    Ok(StatusCode::NO_CONTENT)
}

/// Export the full (unmasked) xpub for a wallet.
#[utoipa::path(
    get,
    path = "/wallets/{wallet_id}/xpub",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(("wallet_id" = Uuid, Path, description = "Wallet ID")),
    responses(
        (status = 200, description = "Full xpub export", body = WalletXpubResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "No such wallet on this account"),
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
    let wallet = owned_wallet(&state, &user, wallet_id).await?;

    Ok(Json(WalletXpubResponse {
        id: wallet.id,
        user_id: wallet.user_id,
        xpub: wallet.xpub,
        derivation_index: wallet.derivation_index,
        name: wallet.name,
        created_at: wallet.created_at,
    }))
}

/// List derived addresses for a wallet.
///
/// Addresses below the wallet's current index have been handed out.
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
        (status = 404, description = "No such wallet on this account"),
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
    let wallet = owned_wallet(&state, &user, wallet_id).await?;

    let limit = query.limit.unwrap_or(20).min(100);
    let offset = query.offset.unwrap_or(0);

    let deriver =
        XpubDeriver::from_xpub(&wallet.xpub).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut addresses = Vec::with_capacity(limit as usize);
    for i in offset..offset.saturating_add(limit) {
        let address = deriver
            .derive_address(i)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        addresses.push(DerivedAddressEntry {
            address: address.to_string(),
            index: i,
            derivation_path: format!("m/44'/60'/0'/0/{i}"),
            used: (i as i64) < wallet.derivation_index as i64,
        });
    }

    Ok(Json(WalletAddressesResponse {
        wallet_id: wallet.id,
        derivation_index: wallet.derivation_index,
        addresses,
    }))
}

/// Get the wallet a store derives from.
#[utoipa::path(
    get,
    path = "/stores/{store_id}/wallet",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(("store_id" = Uuid, Path, description = "Store ID")),
    responses(
        (status = 200, description = "Resolved wallet", body = StoreWalletResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Store has no wallet and the account has no primary"),
    )
)]
pub async fn get_store_wallet<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
) -> Result<Json<StoreWalletResponse>, StatusCode>
where
    A: SessionService + 'static,
{
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

    let wallet = WalletReader::resolve_store_wallet(&*state.data_service, store_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_override = WalletReader::get_store_wallet_override(&*state.data_service, store_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some();

    Ok(Json(StoreWalletResponse {
        store_id,
        wallet: wallet.into(),
        is_override,
    }))
}

/// Pin a store to one of the account's wallets.
#[utoipa::path(
    put,
    path = "/stores/{store_id}/wallet",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(("store_id" = Uuid, Path, description = "Store ID")),
    request_body = SetStoreWalletRequest,
    responses(
        (status = 200, description = "Override set", body = StoreWalletResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Store or wallet not found"),
    )
)]
pub async fn configure_store_wallet<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
    Json(req): Json<SetStoreWalletRequest>,
) -> Result<Json<StoreWalletResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    require_store_settings_permission(&state, &user, store_id).await?;

    let _ = state
        .data_service
        .get_store(StoreId(store_id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // The repository refuses a wallet belonging to another account, so no
    // ownership check is duplicated here.
    WalletWriter::set_store_wallet(&*state.data_service, store_id, req.wallet_id)
        .await
        .map_err(|e| match e {
            data_service::RepositoryError::NotFound(_) => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        })?;

    let wallet = WalletReader::get_wallet(&*state.data_service, req.wallet_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(StoreWalletResponse {
        store_id,
        wallet: wallet.into(),
        is_override: true,
    }))
}

/// Drop a store's override so it follows the account primary again.
///
/// This no longer deletes a wallet - it only stops pinning one. The xpub, its
/// counter and the addresses derived from it are untouched, which is the point:
/// under the old per-store wallet, "delete" destroyed the counter and the next
/// configuration started again at index 0.
#[utoipa::path(
    delete,
    path = "/stores/{store_id}/wallet",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(("store_id" = Uuid, Path, description = "Store ID")),
    responses(
        (status = 204, description = "Override cleared"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
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
    require_store_settings_permission(&state, &user, store_id).await?;

    WalletWriter::clear_store_wallet(&*state.data_service, store_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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

/// Rotate the xpub a store's payment methods derive from.
///
/// Points every payment method for this store at the account wallet holding
/// the new xpub, creating it if the account does not already have it. Old
/// addresses remain watched until their parent invoices resolve, and a
/// rotation record is kept per payment method.
///
/// Note what no longer happens: derivation indices are not reset to zero. The
/// destination wallet carries its own position, so an xpub the account has
/// used before resumes where it left off instead of re-issuing addresses that
/// may already hold funds (RCS-234).
#[utoipa::path(
    post,
    path = "/stores/{store_id}/wallet/rotate",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(("store_id" = Uuid, Path, description = "Store ID")),
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
