//! Store CRUD endpoints: list, create, get, update, delete.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use auth::repository::{StoreRepository, StoreRoleRepository, UserStoreRepository};
use auth::{SessionService, Store, StoreId};

use super::super::extractors::AuthenticatedUser;
use crate::metrics;
use crate::state::PgAppState;

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

    let user_store = auth::UserStore::new(owner_id, store.id, owner_role.id);
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
