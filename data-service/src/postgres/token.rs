//! PostgreSQL implementation of the TokenRepository trait.

use async_trait::async_trait;
use sqlx::Row;
use types::{Network, RepositoryResult, TokenData, TokenQueryParams, TokenReader, TokenWriter};

use super::PgDataService;
use super::conversions::{try_db_to_network, try_network_to_db};
use crate::RepositoryError;
use crate::sqlx_to_repo_error;

/// Convert a database row to TokenData.
fn row_to_token(row: &sqlx::postgres::PgRow) -> Result<TokenData, RepositoryError> {
    let network_str: &str = row.try_get("network").map_err(sqlx_to_repo_error)?;
    let network = try_db_to_network(network_str)?;

    let token_type: String = row.try_get("token_type").map_err(sqlx_to_repo_error)?;
    let decimals: Option<i16> = row.try_get("decimals").map_err(sqlx_to_repo_error)?;

    Ok(TokenData {
        id: Some(row.try_get("id").map_err(sqlx_to_repo_error)?),
        token_type,
        address: row.try_get("address").map_err(sqlx_to_repo_error)?,
        network,
        enabled: row.try_get("enabled").map_err(sqlx_to_repo_error)?,
        name: row.try_get("name").map_err(sqlx_to_repo_error)?,
        symbol: row.try_get("symbol").map_err(sqlx_to_repo_error)?,
        decimals: decimals.map(|d| d as u8),
        token_id: row.try_get("token_id").map_err(sqlx_to_repo_error)?,
    })
}

#[async_trait]
impl TokenReader for PgDataService {
    async fn get(&self, id: i64) -> RepositoryResult<Option<TokenData>> {
        let row = sqlx::query(
            r#"
            SELECT id, token_type, address, network, enabled, name, symbol, decimals, token_id
            FROM tokens
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        match row {
            Some(row) => Ok(Some(row_to_token(&row)?)),
            None => Ok(None),
        }
    }

    async fn get_by_address(
        &self,
        network: Network,
        address: &str,
    ) -> RepositoryResult<Option<TokenData>> {
        let network_str = try_network_to_db(network)?;

        // For ERC20, token_id is NULL. For others, we get the first match.
        let row = sqlx::query(
            r#"
            SELECT id, token_type, address, network, enabled, name, symbol, decimals, token_id
            FROM tokens
            WHERE network = $1 AND LOWER(address) = LOWER($2)
            ORDER BY token_id NULLS FIRST
            LIMIT 1
            "#,
        )
        .bind(network_str)
        .bind(address)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        match row {
            Some(row) => Ok(Some(row_to_token(&row)?)),
            None => Ok(None),
        }
    }

    async fn find_by_symbol(
        &self,
        network: Network,
        symbol: &str,
    ) -> RepositoryResult<Option<TokenData>> {
        let network_str = try_network_to_db(network)?;

        let row = sqlx::query(
            r#"
            SELECT id, token_type, address, network, enabled, name, symbol, decimals, token_id
            FROM tokens
            WHERE network = $1 AND LOWER(symbol) = LOWER($2)
            LIMIT 1
            "#,
        )
        .bind(network_str)
        .bind(symbol)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        match row {
            Some(row) => Ok(Some(row_to_token(&row)?)),
            None => Ok(None),
        }
    }

    async fn query(&self, params: &TokenQueryParams) -> RepositoryResult<(i64, Vec<TokenData>)> {
        // Build dynamic query
        let mut conditions = Vec::new();
        let mut bind_idx = 1;

        if params.token_type.is_some() {
            conditions.push(format!("token_type = ${}", bind_idx));
            bind_idx += 1;
        }
        if params.network.is_some() {
            conditions.push(format!("network = ${}", bind_idx));
            bind_idx += 1;
        }
        if params.enabled.is_some() {
            conditions.push(format!("enabled = ${}", bind_idx));
            bind_idx += 1;
        }
        if params.symbol.is_some() {
            conditions.push(format!("LOWER(symbol) = LOWER(${})", bind_idx));
            bind_idx += 1;
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        // Count query
        let count_sql = format!("SELECT COUNT(*) as count FROM tokens {}", where_clause);

        // Data query
        let data_sql = format!(
            r#"
            SELECT id, token_type, address, network, enabled, name, symbol, decimals, token_id
            FROM tokens
            {}
            ORDER BY network, token_type, symbol NULLS LAST
            LIMIT ${} OFFSET ${}
            "#,
            where_clause,
            bind_idx,
            bind_idx + 1
        );

        // Execute count query
        let mut count_query = sqlx::query(&count_sql);
        if let Some(token_type) = &params.token_type {
            count_query = count_query.bind(token_type);
        }
        if let Some(network) = &params.network {
            count_query = count_query.bind(try_network_to_db(*network)?);
        }
        if let Some(enabled) = &params.enabled {
            count_query = count_query.bind(enabled);
        }
        if let Some(symbol) = &params.symbol {
            count_query = count_query.bind(symbol);
        }

        let count_row = count_query
            .fetch_one(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;
        let total: i64 = count_row.try_get("count").map_err(sqlx_to_repo_error)?;

        // Execute data query
        let mut data_query = sqlx::query(&data_sql);
        if let Some(token_type) = &params.token_type {
            data_query = data_query.bind(token_type);
        }
        if let Some(network) = &params.network {
            data_query = data_query.bind(try_network_to_db(*network)?);
        }
        if let Some(enabled) = &params.enabled {
            data_query = data_query.bind(enabled);
        }
        if let Some(symbol) = &params.symbol {
            data_query = data_query.bind(symbol);
        }
        data_query = data_query.bind(params.limit).bind(params.offset);

        let rows = data_query
            .fetch_all(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        let tokens: Result<Vec<TokenData>, RepositoryError> =
            rows.iter().map(row_to_token).collect();

        Ok((total, tokens?))
    }

    async fn get_enabled_for_network(&self, network: Network) -> RepositoryResult<Vec<TokenData>> {
        let network_str = try_network_to_db(network)?;

        let rows = sqlx::query(
            r#"
            SELECT id, token_type, address, network, enabled, name, symbol, decimals, token_id
            FROM tokens
            WHERE network = $1 AND enabled = true
            ORDER BY token_type, symbol NULLS LAST
            "#,
        )
        .bind(network_str)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        let tokens: Result<Vec<TokenData>, RepositoryError> =
            rows.iter().map(row_to_token).collect();

        tokens
    }
}

#[async_trait]
impl TokenWriter for PgDataService {
    async fn insert(&self, token: &TokenData) -> RepositoryResult<i64> {
        let network_str = try_network_to_db(token.network)?;

        let row = sqlx::query(
            r#"
            INSERT INTO tokens (token_type, address, network, enabled, name, symbol, decimals, token_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING id
            "#,
        )
        .bind(&token.token_type)
        .bind(&token.address)
        .bind(network_str)
        .bind(token.enabled)
        .bind(&token.name)
        .bind(&token.symbol)
        .bind(token.decimals.map(|d| d as i16))
        .bind(&token.token_id)
        .fetch_one(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        let id: i64 = row.try_get("id").map_err(sqlx_to_repo_error)?;
        Ok(id)
    }

    async fn update(&self, token: &TokenData) -> RepositoryResult<()> {
        let id = token.id.ok_or_else(|| {
            RepositoryError::InvalidData("Token must have an ID for update".into())
        })?;

        let network_str = try_network_to_db(token.network)?;

        let result = sqlx::query(
            r#"
            UPDATE tokens
            SET token_type = $1, address = $2, network = $3, enabled = $4,
                name = $5, symbol = $6, decimals = $7, token_id = $8
            WHERE id = $9
            "#,
        )
        .bind(&token.token_type)
        .bind(&token.address)
        .bind(network_str)
        .bind(token.enabled)
        .bind(&token.name)
        .bind(&token.symbol)
        .bind(token.decimals.map(|d| d as i16))
        .bind(&token.token_id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound(format!("Token {} not found", id)));
        }

        Ok(())
    }

    async fn delete(&self, id: i64) -> RepositoryResult<()> {
        let result = sqlx::query("DELETE FROM tokens WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound(format!("Token {} not found", id)));
        }

        Ok(())
    }

    async fn set_enabled(&self, id: i64, enabled: bool) -> RepositoryResult<()> {
        let result = sqlx::query("UPDATE tokens SET enabled = $1 WHERE id = $2")
            .bind(enabled)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound(format!("Token {} not found", id)));
        }

        Ok(())
    }
}
