//! Token management endpoints.
//!
//! All endpoints require admin authentication.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use auth::AuthRepository;
use serde::{Deserialize, Serialize};
use types::{Network, TokenData, TokenQueryParams, TokenReader, TokenWriter};
use utoipa::{IntoParams, ToSchema};

use super::{extractors::AdminAuth, EvmDataService, EvmDataServiceReader, EvmState};
use crate::EvmTokenStandard;

/// Request to create a new token.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTokenRequest {
    /// Token standard (erc20, erc721, erc1155).
    pub standard: String,
    /// Contract address.
    pub address: String,
    /// Network name.
    pub network: String,
    /// Token name.
    pub name: Option<String>,
    /// Token symbol.
    pub symbol: Option<String>,
    /// Number of decimals (required for ERC20).
    pub decimals: Option<u8>,
    /// Token ID (required for ERC1155).
    pub token_id: Option<String>,
    /// Whether the token is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

/// Request to update a token.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateTokenRequest {
    /// Token name.
    pub name: Option<String>,
    /// Token symbol.
    pub symbol: Option<String>,
    /// Number of decimals.
    pub decimals: Option<u8>,
    /// Whether the token is enabled.
    pub enabled: Option<bool>,
}

/// Request to set token enabled status.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetEnabledRequest {
    pub enabled: bool,
}

/// Query parameters for listing tokens.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListTokensQuery {
    /// Filter by token type.
    pub token_type: Option<String>,
    /// Filter by network.
    pub network: Option<String>,
    /// Filter by enabled status.
    pub enabled: Option<bool>,
    /// Filter by symbol.
    pub symbol: Option<String>,
    /// Maximum results (default 100).
    #[serde(default = "default_limit")]
    pub limit: i64,
    /// Offset for pagination.
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    100
}

/// Token response.
#[derive(Debug, Serialize, ToSchema)]
pub struct TokenResponse {
    pub id: i64,
    pub token_type: String,
    pub address: String,
    pub network: String,
    pub enabled: bool,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub decimals: Option<u8>,
    pub token_id: Option<String>,
}

impl From<TokenData> for TokenResponse {
    fn from(t: TokenData) -> Self {
        Self {
            id: t.id.unwrap_or(0),
            token_type: t.token_type,
            address: t.address,
            network: t.network.to_string(),
            enabled: t.enabled,
            name: t.name,
            symbol: t.symbol,
            decimals: t.decimals,
            token_id: t.token_id,
        }
    }
}

/// Token list response with pagination.
#[derive(Debug, Serialize, ToSchema)]
pub struct TokenListResponse {
    pub total: i64,
    pub tokens: Vec<TokenResponse>,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, String)>;

fn parse_network(s: &str) -> Result<Network, (StatusCode, String)> {
    s.parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, format!("invalid network: {}", s)))
}

fn validate_token_standard(
    standard: &str,
    decimals: Option<u8>,
    token_id: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    let std: EvmTokenStandard = standard
        .parse()
        .map_err(|e: String| (StatusCode::BAD_REQUEST, e))?;

    std.validate(decimals, token_id)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[utoipa::path(
    get,
    path = "/evm/tokens",
    tag = "tokens",
    params(ListTokensQuery),
    security(("bearer" = [])),
    responses(
        (status = 200, body = TokenListResponse),
        (status = 400, description = "Invalid query parameters"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin access required"),
    )
)]
pub async fn list_tokens<D, R>(
    _admin: AdminAuth,
    State(state): State<EvmState<D, R>>,
    Query(query): Query<ListTokensQuery>,
) -> ApiResult<TokenListResponse>
where
    D: EvmDataServiceReader + 'static,
    R: AuthRepository + Send + Sync + 'static,
{
    let mut params = TokenQueryParams::default()
        .with_limit(query.limit)
        .with_offset(query.offset);

    if let Some(token_type) = query.token_type {
        params = params.with_token_type(token_type);
    }
    if let Some(network) = query.network {
        let net = parse_network(&network)?;
        params = params.with_network(net);
    }
    if let Some(enabled) = query.enabled {
        params = params.with_enabled(enabled);
    }
    if let Some(symbol) = query.symbol {
        params = params.with_symbol(symbol);
    }

    let (total, tokens) = TokenReader::query(&*state.data_service, &params)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(TokenListResponse {
        total,
        tokens: tokens.into_iter().map(TokenResponse::from).collect(),
    }))
}

#[utoipa::path(
    get,
    path = "/evm/tokens/{id}",
    tag = "tokens",
    params(("id" = i64, Path)),
    security(("bearer" = [])),
    responses(
        (status = 200, body = TokenResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin access required"),
        (status = 404, description = "Token not found"),
    )
)]
pub async fn get_token<D, R>(
    _admin: AdminAuth,
    State(state): State<EvmState<D, R>>,
    Path(id): Path<i64>,
) -> ApiResult<TokenResponse>
where
    D: EvmDataServiceReader + 'static,
    R: AuthRepository + Send + Sync + 'static,
{
    let token = TokenReader::get(&*state.data_service, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("token {} not found", id)))?;

    Ok(Json(TokenResponse::from(token)))
}

#[utoipa::path(
    post,
    path = "/evm/tokens",
    tag = "tokens",
    request_body = CreateTokenRequest,
    security(("bearer" = [])),
    responses(
        (status = 201, body = TokenResponse),
        (status = 400, description = "Invalid token data"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin access required"),
    )
)]
pub async fn create_token<D, R>(
    _admin: AdminAuth,
    State(state): State<EvmState<D, R>>,
    Json(req): Json<CreateTokenRequest>,
) -> Result<(StatusCode, Json<TokenResponse>), (StatusCode, String)>
where
    D: EvmDataService + 'static,
    R: AuthRepository + Send + Sync + 'static,
{
    // Validate network
    let network = parse_network(&req.network)?;

    // Validate EVM network
    if !network.is_evm() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("{} is not an EVM network", req.network),
        ));
    }

    // Validate token standard
    validate_token_standard(&req.standard, req.decimals, req.token_id.as_deref())?;

    // Build token data
    let mut token = TokenData::new(&req.standard, &req.address, network).with_enabled(req.enabled);

    if let Some(name) = req.name {
        token = token.with_name(name);
    }
    if let Some(symbol) = req.symbol {
        token = token.with_symbol(symbol);
    }
    if let Some(decimals) = req.decimals {
        token = token.with_decimals(decimals);
    }
    if let Some(token_id) = req.token_id {
        token = token.with_token_id(token_id);
    }

    // Insert
    let id = TokenWriter::insert(&*state.data_service, &token)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    token.id = Some(id);
    Ok((StatusCode::CREATED, Json(TokenResponse::from(token))))
}

#[utoipa::path(
    put,
    path = "/evm/tokens/{id}",
    tag = "tokens",
    params(("id" = i64, Path)),
    request_body = UpdateTokenRequest,
    security(("bearer" = [])),
    responses(
        (status = 200, body = TokenResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin access required"),
        (status = 404, description = "Token not found"),
    )
)]
pub async fn update_token<D, R>(
    _admin: AdminAuth,
    State(state): State<EvmState<D, R>>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateTokenRequest>,
) -> ApiResult<TokenResponse>
where
    D: EvmDataService + 'static,
    R: AuthRepository + Send + Sync + 'static,
{
    // Get existing token
    let mut token = TokenReader::get(&*state.data_service, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("token {} not found", id)))?;

    // Apply updates
    if let Some(name) = req.name {
        token.name = Some(name);
    }
    if let Some(symbol) = req.symbol {
        token.symbol = Some(symbol);
    }
    if let Some(decimals) = req.decimals {
        token.decimals = Some(decimals);
    }
    if let Some(enabled) = req.enabled {
        token.enabled = enabled;
    }

    // Update
    TokenWriter::update(&*state.data_service, &token)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok(Json(TokenResponse::from(token)))
}

#[utoipa::path(
    delete,
    path = "/evm/tokens/{id}",
    tag = "tokens",
    params(("id" = i64, Path)),
    security(("bearer" = [])),
    responses(
        (status = 204, description = "Token deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin access required"),
        (status = 404, description = "Token not found"),
    )
)]
pub async fn delete_token<D, R>(
    _admin: AdminAuth,
    State(state): State<EvmState<D, R>>,
    Path(id): Path<i64>,
) -> Result<StatusCode, (StatusCode, String)>
where
    D: EvmDataService + 'static,
    R: AuthRepository + Send + Sync + 'static,
{
    TokenWriter::delete(&*state.data_service, id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    put,
    path = "/evm/tokens/{id}/enabled",
    tag = "tokens",
    params(("id" = i64, Path)),
    request_body = SetEnabledRequest,
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Token enabled status updated"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin access required"),
        (status = 404, description = "Token not found"),
    )
)]
pub async fn set_token_enabled<D, R>(
    _admin: AdminAuth,
    State(state): State<EvmState<D, R>>,
    Path(id): Path<i64>,
    Json(req): Json<SetEnabledRequest>,
) -> Result<StatusCode, (StatusCode, String)>
where
    D: EvmDataService + 'static,
    R: AuthRepository + Send + Sync + 'static,
{
    TokenWriter::set_enabled(&*state.data_service, id, req.enabled)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(StatusCode::OK)
}
