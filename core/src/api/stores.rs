//! Store management API endpoints.
//!
//! All endpoints require authentication. Store-level permissions are checked
//! for operations on specific stores.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use auth::{
    AuthRepository, Store, StoreId, UserStore, UserId,
    repository::{StoreRepository, StoreRoleRepository, UserStoreRepository},
};

use crate::state::AppState;
use super::extractors::AuthenticatedUser;

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
pub async fn list_stores<R>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<AppState<R>>,
) -> Result<Json<Vec<StoreResponse>>, StatusCode>
where
    R: AuthRepository + Send + Sync + 'static,
{
    let stores = state.data_service
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
pub async fn create_store<R>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<AppState<R>>,
    Json(req): Json<CreateStoreRequest>,
) -> Result<(StatusCode, Json<StoreResponse>), StatusCode>
where
    R: AuthRepository + Send + Sync + 'static,
{
    let owner_id = user.id;

    let mut store = Store::new(&req.name, owner_id);
    if let Some(website) = req.website {
        store = store.with_website(&website);
    }

    // Create the store
    state.data_service
        .create_store(&store)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Get the Owner role and add owner as member
    let owner_role = state.data_service
        .get_default_role_by_name("Owner")
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    let user_store = UserStore::new(owner_id, store.id, owner_role.id);
    state.data_service
        .add_user_to_store(&user_store)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
pub async fn get_store<R>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<AppState<R>>,
    Path(store_id): Path<Uuid>,
) -> Result<Json<StoreResponse>, StatusCode>
where
    R: AuthRepository + Send + Sync + 'static,
{
    let store = state.data_service
        .get_store(StoreId(store_id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Check user has access (is owner or member)
    let is_owner = store.owner_id == user.id;
    let is_member = state.data_service
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
pub async fn update_store<R>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<AppState<R>>,
    Path(store_id): Path<Uuid>,
    Json(req): Json<UpdateStoreRequest>,
) -> Result<Json<StoreResponse>, StatusCode>
where
    R: AuthRepository + Send + Sync + 'static,
{
    // Check permission
    let has_permission = state.data_service
        .user_has_store_permission(user.id, StoreId(store_id), "ethpay.store.canmodifystoresettings")
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !has_permission {
        return Err(StatusCode::FORBIDDEN);
    }

    let mut store = state.data_service
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

    state.data_service
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
pub async fn delete_store<R>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<AppState<R>>,
    Path(store_id): Path<Uuid>,
) -> Result<StatusCode, StatusCode>
where
    R: AuthRepository + Send + Sync + 'static,
{
    // Only owner can delete
    let store = state.data_service
        .get_store(StoreId(store_id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if store.owner_id != user.id {
        return Err(StatusCode::FORBIDDEN);
    }

    state.data_service
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
pub async fn list_store_members<R>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<AppState<R>>,
    Path(store_id): Path<Uuid>,
) -> Result<Json<Vec<MemberResponse>>, StatusCode>
where
    R: AuthRepository + Send + Sync + 'static,
{
    // Check permission
    let has_permission = state.data_service
        .user_has_store_permission(user.id, StoreId(store_id), "ethpay.store.canviewstoreusers")
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !has_permission {
        return Err(StatusCode::FORBIDDEN);
    }

    let user_stores = state.data_service
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
pub async fn add_store_member<R>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<AppState<R>>,
    Path(store_id): Path<Uuid>,
    Json(req): Json<AddMemberRequest>,
) -> Result<(StatusCode, Json<MemberResponse>), StatusCode>
where
    R: AuthRepository + Send + Sync + 'static,
{
    // Check permission
    let has_permission = state.data_service
        .user_has_store_permission(user.id, StoreId(store_id), "ethpay.store.canmodifystoreusers")
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !has_permission {
        return Err(StatusCode::FORBIDDEN);
    }

    // Verify store exists
    let _ = state.data_service
        .get_store(StoreId(store_id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Get the role by name
    let role = state.data_service
        .get_default_role_by_name(&req.role)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::BAD_REQUEST)?;

    let user_store = UserStore::new(
        UserId(req.user_id),
        StoreId(store_id),
        role.id,
    );

    state.data_service
        .add_user_to_store(&user_store)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok((StatusCode::CREATED, Json(MemberResponse {
        user_id: req.user_id,
        store_id,
        role_id: role.id.0,
        role_name: role.role,
        permissions: role.permissions,
    })))
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
pub async fn update_store_member<R>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<AppState<R>>,
    Path((store_id, target_user_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateMemberRequest>,
) -> Result<Json<MemberResponse>, StatusCode>
where
    R: AuthRepository + Send + Sync + 'static,
{
    // Check permission
    let has_permission = state.data_service
        .user_has_store_permission(user.id, StoreId(store_id), "ethpay.store.canmodifystoreusers")
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !has_permission {
        return Err(StatusCode::FORBIDDEN);
    }

    // Get the new role
    let role = state.data_service
        .get_default_role_by_name(&req.role)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::BAD_REQUEST)?;

    let user_store = UserStore::new(
        UserId(target_user_id),
        StoreId(store_id),
        role.id,
    );

    state.data_service
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
pub async fn remove_store_member<R>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<AppState<R>>,
    Path((store_id, target_user_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, StatusCode>
where
    R: AuthRepository + Send + Sync + 'static,
{
    // Check permission
    let has_permission = state.data_service
        .user_has_store_permission(user.id, StoreId(store_id), "ethpay.store.canmodifystoreusers")
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !has_permission {
        return Err(StatusCode::FORBIDDEN);
    }

    // Cannot remove the store owner
    let store = state.data_service
        .get_store(StoreId(store_id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if store.owner_id.0 == target_user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    state.data_service
        .remove_user_from_store(UserId(target_user_id), StoreId(store_id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}
