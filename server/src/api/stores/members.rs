//! Store member management endpoints: list, add, update, remove.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use auth::repository::{StoreRepository, StoreRoleRepository, UserStoreRepository};
use auth::{SessionService, StoreId, UserId, UserStore};

use super::super::extractors::AuthenticatedUser;
use crate::state::PgAppState;

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
