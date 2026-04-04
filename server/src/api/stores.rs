//! Store management API endpoints.
//!
//! All endpoints require authentication. Store-level permissions are checked
//! for operations on specific stores.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use auth::{
    SessionService, Store, StoreId, UserId, UserStore,
    repository::{StoreRepository, StoreRoleRepository, UserStoreRepository},
};
use data_service::{
    StorePaymentMethod, StorePaymentMethodReader, StorePaymentMethodWriter, StoreWalletReader,
    StoreWalletWriter,
};
use evm::validate_xpub;
use types::{StoreWebhookReader, StoreWebhookWriter};

use super::extractors::AuthenticatedUser;
use crate::metrics;
use crate::state::PgAppState;

/// Check if user can modify store settings (admin or has permission).
async fn require_store_settings_permission<A: SessionService>(
    state: &PgAppState<A>,
    user: &auth::UserInfo,
    store_id: Uuid,
) -> Result<(), StatusCode> {
    if user.role == auth::Role::ServerAdmin {
        return Ok(());
    }

    let has_permission = state
        .data_service
        .user_has_store_permission(
            user.id,
            StoreId(store_id),
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

/// Request to create a new store.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateStoreRequest {
    /// Store name.
    pub name: String,
    /// Optional website URL.
    pub website: Option<String>,
}

/// Request to update a store.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateStoreRequest {
    /// New store name.
    pub name: Option<String>,
    /// New website URL.
    pub website: Option<String>,
}

/// Store response.
#[derive(Debug, Serialize, ToSchema)]
pub struct StoreResponse {
    /// Store ID.
    pub id: Uuid,
    /// Store name.
    pub name: String,
    /// Website URL.
    pub website: Option<String>,
    /// Owner user ID.
    pub owner_id: Uuid,
    /// Whether the store is archived.
    pub archived: bool,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<Store> for StoreResponse {
    fn from(store: Store) -> Self {
        Self {
            id: store.id.0,
            name: store.name,
            website: store.website,
            owner_id: store.owner_id.0,
            archived: store.archived,
            created_at: store.created_at,
        }
    }
}

/// Request to add a member to a store.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddMemberRequest {
    /// User ID to add.
    pub user_id: Uuid,
    /// Role name (Owner, Manager, Employee, Guest).
    pub role: String,
}

/// Request to update a member's role.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateMemberRequest {
    /// New role name.
    pub role: String,
}

/// Store member response.
#[derive(Debug, Serialize, ToSchema)]
pub struct MemberResponse {
    /// User ID.
    pub user_id: Uuid,
    /// Store ID.
    pub store_id: Uuid,
    /// Role ID.
    pub role_id: Uuid,
    /// Role name.
    pub role_name: String,
    /// Role permissions.
    pub permissions: Vec<String>,
}

/// List stores for the authenticated user.
#[utoipa::path(
    get,
    path = "/stores",
    tag = "stores",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "List of stores", body = Vec<StoreResponse>),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn list_stores<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
) -> Result<Json<Vec<StoreResponse>>, StatusCode>
where
    A: SessionService + 'static,
{
    let stores = state
        .data_service
        .get_stores_for_user(user.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(stores.into_iter().map(|s| s.into()).collect()))
}

/// Create a new store.
///
/// The authenticated user becomes the store owner.
#[utoipa::path(
    post,
    path = "/stores",
    tag = "stores",
    security(("bearer_auth" = [])),
    request_body = CreateStoreRequest,
    responses(
        (status = 201, description = "Store created", body = StoreResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn create_store<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Json(req): Json<CreateStoreRequest>,
) -> Result<(StatusCode, Json<StoreResponse>), StatusCode>
where
    A: SessionService + 'static,
{
    let owner_id = user.id;

    let mut store = Store::new(&req.name, owner_id);
    if let Some(website) = req.website {
        store = store.with_website(&website);
    }

    // Create the store
    state
        .data_service
        .create_store(&store)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Get the Owner role and add owner as member
    let owner_role = state
        .data_service
        .get_default_role_by_name("Owner")
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    let user_store = UserStore::new(owner_id, store.id, owner_role.id);
    state
        .data_service
        .add_user_to_store(&user_store)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    metrics::record_store_created();
    Ok((StatusCode::CREATED, Json(store.into())))
}

/// Get a store by ID.
///
/// User must be a member of the store.
#[utoipa::path(
    get,
    path = "/stores/{store_id}",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID")
    ),
    responses(
        (status = 200, description = "Store details", body = StoreResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Not a member of this store"),
        (status = 404, description = "Store not found"),
    )
)]
pub async fn get_store<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
) -> Result<Json<StoreResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let store = state
        .data_service
        .get_store(StoreId(store_id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Check user has access (is owner or member)
    let is_owner = store.owner_id == user.id;
    let is_member = state
        .data_service
        .get_user_store(user.id, StoreId(store_id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some();

    if !is_owner && !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(Json(store.into()))
}

/// Update a store.
///
/// Requires `canmodifystoresettings` permission.
#[utoipa::path(
    put,
    path = "/stores/{store_id}",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID")
    ),
    request_body = UpdateStoreRequest,
    responses(
        (status = 200, description = "Store updated", body = StoreResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Store not found"),
    )
)]
pub async fn update_store<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
    Json(req): Json<UpdateStoreRequest>,
) -> Result<Json<StoreResponse>, StatusCode>
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

    let mut store = state
        .data_service
        .get_store(StoreId(store_id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if let Some(name) = req.name {
        store.name = name;
    }
    if let Some(website) = req.website {
        store.website = Some(website);
    }

    state
        .data_service
        .update_store(&store)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(store.into()))
}

/// Delete (archive) a store.
///
/// Only the store owner can delete a store.
#[utoipa::path(
    delete,
    path = "/stores/{store_id}",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID")
    ),
    responses(
        (status = 204, description = "Store deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Only store owner can delete"),
        (status = 404, description = "Store not found"),
    )
)]
pub async fn delete_store<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
) -> Result<StatusCode, StatusCode>
where
    A: SessionService + 'static,
{
    // Only owner can delete
    let store = state
        .data_service
        .get_store(StoreId(store_id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if store.owner_id != user.id {
        return Err(StatusCode::FORBIDDEN);
    }

    state
        .data_service
        .archive_store(StoreId(store_id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

/// List members of a store.
///
/// Requires `canviewstoreusers` permission.
#[utoipa::path(
    get,
    path = "/stores/{store_id}/members",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID")
    ),
    responses(
        (status = 200, description = "List of members", body = Vec<MemberResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Store not found"),
    )
)]
pub async fn list_store_members<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
) -> Result<Json<Vec<MemberResponse>>, StatusCode>
where
    A: SessionService + 'static,
{
    // Check permission
    let has_permission = state
        .data_service
        .user_has_store_permission(user.id, StoreId(store_id), "ethpay.store.canviewstoreusers")
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !has_permission {
        return Err(StatusCode::FORBIDDEN);
    }

    let user_stores = state
        .data_service
        .get_store_users(StoreId(store_id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut members = Vec::new();
    for us in user_stores {
        if let Ok(Some(role)) = state.data_service.get_store_role(us.store_role_id).await {
            members.push(MemberResponse {
                user_id: us.user_id.0,
                store_id: us.store_id.0,
                role_id: us.store_role_id.0,
                role_name: role.role,
                permissions: role.permissions,
            });
        }
    }

    Ok(Json(members))
}

/// Add a member to a store.
///
/// Requires `canmodifystoreusers` permission.
#[utoipa::path(
    post,
    path = "/stores/{store_id}/members",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID")
    ),
    request_body = AddMemberRequest,
    responses(
        (status = 201, description = "Member added", body = MemberResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Store or role not found"),
    )
)]
pub async fn add_store_member<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
    Json(req): Json<AddMemberRequest>,
) -> Result<(StatusCode, Json<MemberResponse>), StatusCode>
where
    A: SessionService + 'static,
{
    // Check permission
    let has_permission = state
        .data_service
        .user_has_store_permission(
            user.id,
            StoreId(store_id),
            "ethpay.store.canmodifystoreusers",
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !has_permission {
        return Err(StatusCode::FORBIDDEN);
    }

    // Verify store exists
    let _ = state
        .data_service
        .get_store(StoreId(store_id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Get the role by name
    let role = state
        .data_service
        .get_default_role_by_name(&req.role)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::BAD_REQUEST)?;

    let user_store = UserStore::new(UserId(req.user_id), StoreId(store_id), role.id);

    state
        .data_service
        .add_user_to_store(&user_store)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok((
        StatusCode::CREATED,
        Json(MemberResponse {
            user_id: req.user_id,
            store_id,
            role_id: role.id.0,
            role_name: role.role,
            permissions: role.permissions,
        }),
    ))
}

/// Update a member's role in a store.
///
/// Requires `canmodifystoreusers` permission.
#[utoipa::path(
    put,
    path = "/stores/{store_id}/members/{user_id}",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID"),
        ("user_id" = Uuid, Path, description = "User ID")
    ),
    request_body = UpdateMemberRequest,
    responses(
        (status = 200, description = "Member updated", body = MemberResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Member not found"),
    )
)]
pub async fn update_store_member<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path((store_id, target_user_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateMemberRequest>,
) -> Result<Json<MemberResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    // Check permission
    let has_permission = state
        .data_service
        .user_has_store_permission(
            user.id,
            StoreId(store_id),
            "ethpay.store.canmodifystoreusers",
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !has_permission {
        return Err(StatusCode::FORBIDDEN);
    }

    // Get the new role
    let role = state
        .data_service
        .get_default_role_by_name(&req.role)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::BAD_REQUEST)?;

    let user_store = UserStore::new(UserId(target_user_id), StoreId(store_id), role.id);

    state
        .data_service
        .update_user_store(&user_store)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(Json(MemberResponse {
        user_id: target_user_id,
        store_id,
        role_id: role.id.0,
        role_name: role.role,
        permissions: role.permissions,
    }))
}

/// Remove a member from a store.
///
/// Requires `canmodifystoreusers` permission.
/// Cannot remove the store owner.
#[utoipa::path(
    delete,
    path = "/stores/{store_id}/members/{user_id}",
    tag = "stores",
    security(("bearer_auth" = [])),
    params(
        ("store_id" = Uuid, Path, description = "Store ID"),
        ("user_id" = Uuid, Path, description = "User ID")
    ),
    responses(
        (status = 204, description = "Member removed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions or cannot remove owner"),
        (status = 404, description = "Member not found"),
    )
)]
pub async fn remove_store_member<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path((store_id, target_user_id)): Path<(Uuid, Uuid)>,
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
            "ethpay.store.canmodifystoreusers",
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !has_permission {
        return Err(StatusCode::FORBIDDEN);
    }

    // Cannot remove the store owner
    let store = state
        .data_service
        .get_store(StoreId(store_id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if store.owner_id.0 == target_user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    state
        .data_service
        .remove_user_from_store(UserId(target_user_id), StoreId(store_id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

// =============================================================================
// Store Wallet Configuration
// =============================================================================

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

/// Mask an xpub for display (show first 8 and last 8 chars).
fn mask_xpub(xpub: &str) -> String {
    if xpub.len() <= 20 {
        return "****".to_string();
    }
    format!("{}...{}", &xpub[..8], &xpub[xpub.len() - 8..])
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
// Webhook Management
// =============================================================================

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

// =============================================================================
// Payment Methods
// =============================================================================

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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    // =========================================================================
    // mask_xpub
    // =========================================================================

    #[test]
    fn test_mask_xpub_normal() {
        let xpub = "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz";
        let masked = mask_xpub(xpub);
        assert!(masked.starts_with("xpub6CUG"));
        assert!(masked.contains("..."));
        assert!(masked.ends_with("3fDVmz"));
    }

    #[test]
    fn test_mask_xpub_short() {
        assert_eq!(mask_xpub("short"), "****");
    }

    #[test]
    fn test_mask_xpub_boundary_20() {
        assert_eq!(mask_xpub("12345678901234567890"), "****");
    }

    #[test]
    fn test_mask_xpub_boundary_21() {
        assert_eq!(mask_xpub("123456789012345678901"), "12345678...45678901");
    }

    // =========================================================================
    // StoreResponse conversion
    // =========================================================================

    #[test]
    fn test_store_response_from_store() {
        let now = Utc::now();
        let store = Store {
            id: StoreId(Uuid::nil()),
            name: "Test Store".to_string(),
            website: Some("https://example.com".to_string()),
            owner_id: UserId(Uuid::nil()),
            archived: false,
            created_at: now,
        };

        let response: StoreResponse = store.into();
        assert_eq!(response.id, Uuid::nil());
        assert_eq!(response.name, "Test Store");
        assert_eq!(response.website, Some("https://example.com".to_string()));
        assert_eq!(response.owner_id, Uuid::nil());
        assert!(!response.archived);
        assert_eq!(response.created_at, now);
    }

    #[test]
    fn test_store_response_without_website() {
        let store = Store {
            id: StoreId(Uuid::new_v4()),
            name: "No Website".to_string(),
            website: None,
            owner_id: UserId(Uuid::new_v4()),
            archived: false,
            created_at: Utc::now(),
        };

        let response: StoreResponse = store.into();
        assert!(response.website.is_none());
    }

    #[test]
    fn test_store_response_archived() {
        let store = Store {
            id: StoreId(Uuid::new_v4()),
            name: "Archived".to_string(),
            website: None,
            owner_id: UserId(Uuid::new_v4()),
            archived: true,
            created_at: Utc::now(),
        };

        let response: StoreResponse = store.into();
        assert!(response.archived);
    }

    // =========================================================================
    // PaymentMethodResponse conversion
    // =========================================================================

    #[test]
    fn test_payment_method_response_native() {
        let pm = StorePaymentMethod {
            id: Uuid::nil(),
            store_id: Uuid::nil(),
            chain_id: 1,
            token_address: None,
            asset_symbol: "ETH".to_string(),
            decimals: 18,
            xpub: "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz".to_string(),
            derivation_index: 5,
            enabled: true,
            created_at: Utc::now(),
        };

        let response: PaymentMethodResponse = pm.into();
        assert_eq!(response.chain_id, 1);
        assert_eq!(response.asset_symbol, "ETH");
        assert!(response.token_address.is_none());
        assert_eq!(response.derivation_index, 5);
        assert!(response.enabled);
        assert!(response.xpub_masked.contains("..."));
    }

    #[test]
    fn test_payment_method_response_erc20() {
        let token_addr = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string();
        let pm = StorePaymentMethod {
            id: Uuid::nil(),
            store_id: Uuid::nil(),
            chain_id: 137,
            token_address: Some(token_addr.clone()),
            asset_symbol: "USDC".to_string(),
            decimals: 6,
            xpub: "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz".to_string(),
            derivation_index: 0,
            enabled: false,
            created_at: Utc::now(),
        };

        let response: PaymentMethodResponse = pm.into();
        assert_eq!(response.chain_id, 137);
        assert_eq!(response.asset_symbol, "USDC");
        assert_eq!(response.token_address, Some(token_addr));
        assert!(!response.enabled);
    }

    // =========================================================================
    // Request deserialization
    // =========================================================================

    #[test]
    fn test_create_store_request() {
        let json = r#"{"name": "My Store", "website": "https://mystore.com"}"#;
        let req: CreateStoreRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "My Store");
        assert_eq!(req.website, Some("https://mystore.com".to_string()));
    }

    #[test]
    fn test_create_store_request_without_website() {
        let json = r#"{"name": "My Store"}"#;
        let req: CreateStoreRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "My Store");
        assert!(req.website.is_none());
    }

    #[test]
    fn test_create_store_request_missing_name() {
        let json = r#"{"website": "https://mystore.com"}"#;
        let result = serde_json::from_str::<CreateStoreRequest>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_update_store_request_partial() {
        let json = r#"{"name": "New Name"}"#;
        let req: UpdateStoreRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, Some("New Name".to_string()));
        assert!(req.website.is_none());
    }

    #[test]
    fn test_update_store_request_empty() {
        let json = r#"{}"#;
        let req: UpdateStoreRequest = serde_json::from_str(json).unwrap();
        assert!(req.name.is_none());
        assert!(req.website.is_none());
    }

    #[test]
    fn test_create_payment_method_request() {
        let json = r#"{
            "chain_id": 1,
            "token_address": null,
            "asset_symbol": "ETH",
            "decimals": 18,
            "xpub": "xpub123..."
        }"#;
        let req: CreatePaymentMethodRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.chain_id, 1);
        assert!(req.token_address.is_none());
        assert_eq!(req.asset_symbol, "ETH");
        assert_eq!(req.decimals, 18);
    }

    #[test]
    fn test_update_payment_method_request_partial() {
        let json = r#"{"enabled": false}"#;
        let req: UpdatePaymentMethodRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.enabled, Some(false));
        assert!(req.xpub.is_none());
    }

    #[test]
    fn test_configure_webhook_request_defaults_enabled() {
        let json = r#"{"webhook_url": "https://example.com/hook"}"#;
        let req: ConfigureWebhookRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.webhook_url, "https://example.com/hook");
        assert!(req.enabled);
    }

    #[test]
    fn test_configure_webhook_request_disabled() {
        let json = r#"{"webhook_url": "https://example.com/hook", "enabled": false}"#;
        let req: ConfigureWebhookRequest = serde_json::from_str(json).unwrap();
        assert!(!req.enabled);
    }

    // =========================================================================
    // Response serialization
    // =========================================================================

    #[test]
    fn test_store_response_json() {
        let response = StoreResponse {
            id: Uuid::nil(),
            name: "Test".to_string(),
            website: None,
            owner_id: Uuid::nil(),
            archived: false,
            created_at: Utc::now(),
        };

        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["name"], "Test");
        assert_eq!(json["archived"], false);
        assert!(json["website"].is_null());
        assert!(json["id"].is_string());
        assert!(json["created_at"].is_string());
    }

    #[test]
    fn test_member_response_json() {
        let response = MemberResponse {
            user_id: Uuid::nil(),
            store_id: Uuid::nil(),
            role_id: Uuid::nil(),
            role_name: "Owner".to_string(),
            permissions: vec![
                "ethpay.store.canmodifystoresettings".to_string(),
                "ethpay.store.canviewinvoices".to_string(),
            ],
        };

        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["role_name"], "Owner");
        assert_eq!(json["permissions"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_wallet_response_json() {
        let response = WalletResponse {
            id: Uuid::nil(),
            store_id: Uuid::nil(),
            xpub_masked: "xpub6CUG...3fDVmz".to_string(),
            derivation_index: 42,
            name: Some("Main Wallet".to_string()),
            created_at: Utc::now(),
        };

        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["derivation_index"], 42);
        assert_eq!(json["name"], "Main Wallet");
        assert_eq!(json["xpub_masked"], "xpub6CUG...3fDVmz");
    }

    #[test]
    fn test_webhook_response_with_secret() {
        let response = WebhookResponse {
            id: Uuid::nil(),
            store_id: Uuid::nil(),
            webhook_url: "https://example.com/webhook".to_string(),
            webhook_secret: Some("secret123".to_string()),
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["webhook_url"], "https://example.com/webhook");
        assert_eq!(json["webhook_secret"], "secret123");
        assert!(json["enabled"].as_bool().unwrap());
    }

    #[test]
    fn test_webhook_response_without_secret() {
        let response = WebhookResponse {
            id: Uuid::nil(),
            store_id: Uuid::nil(),
            webhook_url: "https://example.com/webhook".to_string(),
            webhook_secret: None,
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let json = serde_json::to_value(&response).unwrap();
        assert!(json["webhook_secret"].is_null());
    }

    // =========================================================================
    // list_wallets response shape
    // =========================================================================

    #[test]
    fn test_list_wallets_response_serialization() {
        let wallets = vec![
            WalletResponse {
                id: Uuid::nil(),
                store_id: Uuid::nil(),
                xpub_masked: mask_xpub(
                    "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz",
                ),
                derivation_index: 0,
                name: Some("ETH Wallet".to_string()),
                created_at: Utc::now(),
            },
            WalletResponse {
                id: Uuid::new_v4(),
                store_id: Uuid::new_v4(),
                xpub_masked: mask_xpub(
                    "xpub6D4BDPcP2GT577Vvch3R8wDkScZWzQzMMUm3PWbmWvVJrZwQY4VUNgqFJPMM3No2dFDFGTsxxpG5uJh7n7epu4trkrX7x7DogT5Uv6fcLW5",
                ),
                derivation_index: 5,
                name: None,
                created_at: Utc::now(),
            },
        ];

        let json = serde_json::to_value(&wallets).unwrap();
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["name"], "ETH Wallet");
        assert_eq!(arr[0]["derivation_index"], 0);
        assert!(arr[0]["xpub_masked"].as_str().unwrap().contains("..."));
        assert!(arr[1]["name"].is_null());
        assert_eq!(arr[1]["derivation_index"], 5);
    }

    #[test]
    fn test_list_wallets_empty_response() {
        let wallets: Vec<WalletResponse> = vec![];
        let json = serde_json::to_value(&wallets).unwrap();
        assert!(json.as_array().unwrap().is_empty());
    }
}
