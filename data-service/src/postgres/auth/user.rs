//! UserRepository implementation.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::Row;

use auth::{
    User, UserId, UserRepository,
    error::{AuthError, Result},
};

use super::{PgDataService, sqlx_to_auth_error};

#[async_trait]
impl UserRepository for PgDataService {
    async fn create_user(&self, user: &User) -> Result<()> {
        let kdf_params = serde_json::to_value(&user.kdf_params)
            .map_err(|e| AuthError::Repository(e.to_string()))?;
        let encrypted_key = serde_json::to_value(&user.encrypted_symmetric_key)
            .map_err(|e| AuthError::Repository(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO users (
                id, email, primary_wallet_address, kdf_salt_identifier, kdf_params,
                encrypted_symmetric_key, recovery_verification_hash, created_at,
                last_login_at, failed_login_attempts, locked_until, role
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            "#,
        )
        .bind(user.id.0)
        .bind(&user.email)
        .bind(&user.primary_wallet_address)
        .bind(&user.kdf_salt_identifier)
        .bind(&kdf_params)
        .bind(&encrypted_key)
        .bind(&user.recovery_verification_hash)
        .bind(user.created_at)
        .bind(user.last_login_at)
        .bind(user.failed_login_attempts as i32)
        .bind(user.locked_until)
        .bind(user.role.as_str())
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(())
    }

    async fn get_user(&self, id: UserId) -> Result<Option<User>> {
        let row = sqlx::query(
            r#"
            SELECT id, email, primary_wallet_address, kdf_salt_identifier, kdf_params,
                   encrypted_symmetric_key, recovery_verification_hash, created_at,
                   last_login_at, failed_login_attempts, locked_until, role
            FROM users WHERE id = $1
            "#,
        )
        .bind(id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        row.map(|r| row_to_user(&r)).transpose()
    }

    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>> {
        let row = sqlx::query(
            r#"
            SELECT id, email, primary_wallet_address, kdf_salt_identifier, kdf_params,
                   encrypted_symmetric_key, recovery_verification_hash, created_at,
                   last_login_at, failed_login_attempts, locked_until, role
            FROM users WHERE LOWER(email) = LOWER($1)
            "#,
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        row.map(|r| row_to_user(&r)).transpose()
    }

    async fn get_user_by_wallet_address(&self, address: &str) -> Result<Option<User>> {
        let row = sqlx::query(
            r#"
            SELECT id, email, primary_wallet_address, kdf_salt_identifier, kdf_params,
                   encrypted_symmetric_key, recovery_verification_hash, created_at,
                   last_login_at, failed_login_attempts, locked_until, role
            FROM users WHERE primary_wallet_address = $1
            "#,
        )
        .bind(address)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        row.map(|r| row_to_user(&r)).transpose()
    }

    async fn update_user(&self, user: &User) -> Result<()> {
        let kdf_params = serde_json::to_value(&user.kdf_params)
            .map_err(|e| AuthError::Repository(e.to_string()))?;
        let encrypted_key = serde_json::to_value(&user.encrypted_symmetric_key)
            .map_err(|e| AuthError::Repository(e.to_string()))?;

        let result = sqlx::query(
            r#"
            -- kdf_salt_identifier is deliberately absent: it is pinned at
            -- registration and the stored recovery_verification_hash was
            -- derived from it. Updating it would make the account
            -- unrecoverable, so this statement cannot (RCS-201).
            UPDATE users SET
                email = $2, primary_wallet_address = $3, kdf_params = $4,
                encrypted_symmetric_key = $5, recovery_verification_hash = $6,
                last_login_at = $7, failed_login_attempts = $8, locked_until = $9,
                role = $10
            WHERE id = $1
            "#,
        )
        .bind(user.id.0)
        .bind(&user.email)
        .bind(&user.primary_wallet_address)
        .bind(&kdf_params)
        .bind(&encrypted_key)
        .bind(&user.recovery_verification_hash)
        .bind(user.last_login_at)
        .bind(user.failed_login_attempts as i32)
        .bind(user.locked_until)
        .bind(user.role.as_str())
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        if result.rows_affected() == 0 {
            return Err(AuthError::UserNotFound(user.id.to_string()));
        }
        Ok(())
    }

    async fn delete_user(&self, id: UserId) -> Result<()> {
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_auth_error)?;
        Ok(())
    }

    async fn increment_failed_logins(&self, id: UserId) -> Result<u32> {
        let row = sqlx::query(
            r#"
            UPDATE users SET failed_login_attempts = failed_login_attempts + 1
            WHERE id = $1
            RETURNING failed_login_attempts
            "#,
        )
        .bind(id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        match row {
            Some(r) => {
                let count: i32 = r.get("failed_login_attempts");
                Ok(count as u32)
            }
            None => Err(AuthError::UserNotFound(id.to_string())),
        }
    }

    async fn reset_failed_logins(&self, id: UserId) -> Result<()> {
        let result = sqlx::query("UPDATE users SET failed_login_attempts = 0 WHERE id = $1")
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_auth_error)?;

        if result.rows_affected() == 0 {
            return Err(AuthError::UserNotFound(id.to_string()));
        }
        Ok(())
    }

    async fn lock_user(&self, id: UserId, until: DateTime<Utc>) -> Result<()> {
        let result = sqlx::query("UPDATE users SET locked_until = $2 WHERE id = $1")
            .bind(id.0)
            .bind(until)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_auth_error)?;

        if result.rows_affected() == 0 {
            return Err(AuthError::UserNotFound(id.to_string()));
        }
        Ok(())
    }

    async fn unlock_user(&self, id: UserId) -> Result<()> {
        let result = sqlx::query("UPDATE users SET locked_until = NULL WHERE id = $1")
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_auth_error)?;

        if result.rows_affected() == 0 {
            return Err(AuthError::UserNotFound(id.to_string()));
        }
        Ok(())
    }

    async fn list_users(&self, offset: i64, limit: i64) -> Result<Vec<User>> {
        let rows = sqlx::query(
            r#"
            SELECT id, email, primary_wallet_address, kdf_salt_identifier, kdf_params,
                   encrypted_symmetric_key, recovery_verification_hash, created_at,
                   last_login_at, failed_login_attempts, locked_until, role
            FROM users ORDER BY created_at DESC LIMIT $1 OFFSET $2
            "#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        rows.iter().map(row_to_user).collect()
    }

    async fn count_users(&self) -> Result<i64> {
        let row = sqlx::query("SELECT COUNT(*) as count FROM users")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlx_to_auth_error)?;
        Ok(row.get::<i64, _>("count"))
    }
}

fn row_to_user(row: &sqlx::postgres::PgRow) -> Result<User> {
    let kdf_params_json: serde_json::Value = row.get("kdf_params");
    let encrypted_key_json: serde_json::Value = row.get("encrypted_symmetric_key");
    let failed_attempts: i32 = row.get("failed_login_attempts");
    let role_str: String = row.get("role");

    let id: uuid::Uuid = row.get("id");
    let email: Option<String> = row.get("email");
    let primary_wallet_address: Option<String> = row.get("primary_wallet_address");

    Ok(User {
        id: UserId(id),
        email: email.clone(),
        primary_wallet_address: primary_wallet_address.clone(),
        // NULL means the row predates RCS-201's backfill (or was written by the
        // old binary during a rolling deploy). Fall back to the computed value,
        // which is what such a row was salted with anyway.
        kdf_salt_identifier: row
            .get::<Option<String>, _>("kdf_salt_identifier")
            .unwrap_or_else(|| match (&email, &primary_wallet_address) {
                (_, Some(w)) => format!("wallet:{w}"),
                (Some(e), None) => e.clone(),
                (None, None) => format!("passkey:{id}"),
            }),
        kdf_params: serde_json::from_value(kdf_params_json)
            .map_err(|e| AuthError::Repository(e.to_string()))?,
        encrypted_symmetric_key: serde_json::from_value(encrypted_key_json)
            .map_err(|e| AuthError::Repository(e.to_string()))?,
        recovery_verification_hash: row.get("recovery_verification_hash"),
        created_at: row.get("created_at"),
        last_login_at: row.get("last_login_at"),
        failed_login_attempts: failed_attempts as u32,
        locked_until: row.get("locked_until"),
        role: role_str.parse().unwrap_or_default(),
    })
}
