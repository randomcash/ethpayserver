//! Store token policy endpoints: get, set (upsert), delete.

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
use data_service::{
    StoreTokenPolicyReader, StoreTokenPolicyWriter, TokenPolicyEntryInput, TokenPolicyMode,
};

use super::super::extractors::AuthenticatedUser;
use super::require_store_settings_permission;
use crate::state::PgAppState;

/// Token policy entry for API requests/responses.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TokenPolicyEntryPayload {
    pub chain_id: i64,
    pub token_address: Option<String>,
    pub asset_symbol: String,
}

/// Token policy response.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TokenPolicyResponse {
    pub id: String,
    pub store_id: Uuid,
    pub mode: String,
    pub entries: Vec<TokenPolicyEntryPayload>,
    pub created_at: String,
    pub updated_at: String,
}

/// Request to set a token policy.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetTokenPolicyRequest {
    pub mode: String,
    pub entries: Vec<TokenPolicyEntryPayload>,
}

/// Get store token policy.
pub async fn get_token_policy<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
) -> Result<Json<Option<TokenPolicyResponse>>, StatusCode>
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

    let policy = StoreTokenPolicyReader::get_token_policy(&*state.data_service, store_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(policy.map(|p| {
        TokenPolicyResponse {
            id: p.id.to_string(),
            store_id: p.store_id,
            mode: p.mode.to_string(),
            entries: p
                .entries
                .into_iter()
                .map(|e| TokenPolicyEntryPayload {
                    chain_id: e.chain_id,
                    token_address: e.token_address,
                    asset_symbol: e.asset_symbol,
                })
                .collect(),
            created_at: p.created_at.to_rfc3339(),
            updated_at: p.updated_at.to_rfc3339(),
        }
    })))
}

/// Set (upsert) store token policy.
pub async fn set_token_policy<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
    Json(req): Json<SetTokenPolicyRequest>,
) -> Result<Json<TokenPolicyResponse>, StatusCode>
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

    // Validate mode
    let mode: TokenPolicyMode = req.mode.parse().map_err(|_| StatusCode::BAD_REQUEST)?;

    // Validate entries
    for entry in &req.entries {
        if entry.asset_symbol.is_empty() || entry.asset_symbol.len() > 32 {
            return Err(StatusCode::BAD_REQUEST);
        }
        if let Some(ref addr) = entry.token_address
            && (addr.len() != 42 || !addr.starts_with("0x"))
        {
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    let inputs: Vec<TokenPolicyEntryInput> = req
        .entries
        .into_iter()
        .map(|e| TokenPolicyEntryInput {
            chain_id: e.chain_id,
            token_address: e.token_address,
            asset_symbol: e.asset_symbol,
        })
        .collect();

    let policy =
        StoreTokenPolicyWriter::upsert_token_policy(&*state.data_service, store_id, mode, &inputs)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(TokenPolicyResponse {
        id: policy.id.to_string(),
        store_id: policy.store_id,
        mode: policy.mode.to_string(),
        entries: policy
            .entries
            .into_iter()
            .map(|e| TokenPolicyEntryPayload {
                chain_id: e.chain_id,
                token_address: e.token_address,
                asset_symbol: e.asset_symbol,
            })
            .collect(),
        created_at: policy.created_at.to_rfc3339(),
        updated_at: policy.updated_at.to_rfc3339(),
    }))
}

/// Delete store token policy (revert to accept-all).
pub async fn delete_token_policy<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
) -> Result<StatusCode, StatusCode>
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

    StoreTokenPolicyWriter::delete_token_policy(&*state.data_service, store_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}
