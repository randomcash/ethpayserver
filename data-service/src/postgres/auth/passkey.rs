//! PasskeyRepository implementation.

use async_trait::async_trait;
use sqlx::Row;

use auth::{PasskeyCredential, PasskeyId, PasskeyRepository, UserId, error::{AuthError, Result}};

use super::{PgDataService, sqlx_to_auth_error};

#[async_trait]
impl PasskeyRepository for PgDataService {
    async fn create_passkey(&self, credential: &PasskeyCredential) -> Result<()> {
        let passkey_data = serde_json::to_value(&credential.passkey)
            .map_err(|e| AuthError::Repository(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO passkey_credentials (
                id, user_id, name, passkey, created_at, last_used_at, is_active
            ) VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
        )
        .bind(credential.id.0)
        .bind(credential.user_id.0)
        .bind(&credential.name)
        .bind(&passkey_data)
        .bind(credential.created_at)
        .bind(credential.last_used_at)
        .bind(credential.is_active)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(())
    }

    async fn get_passkey(&self, id: PasskeyId) -> Result<Option<PasskeyCredential>> {
        let row = sqlx::query(
            r#"
            SELECT id, user_id, name, passkey, created_at, last_used_at, is_active
            FROM passkey_credentials WHERE id = $1
            "#,
        )
        .bind(id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        row.map(|r| row_to_passkey(&r)).transpose()
    }

    async fn get_passkey_by_credential_id(&self, credential_id: &[u8]) -> Result<Option<PasskeyCredential>> {
        // The credential ID is stored as base64 in the passkey JSON under "cred" -> "cred_id"
        // We need to encode our bytes to base64 and search for it
        use base64::Engine;
        let cred_id_b64 = base64::engine::general_purpose::STANDARD.encode(credential_id);

        // Query for active passkeys where the JSON cred_id matches
        // The passkey JSON structure is: { "cred": { "cred_id": "base64..." }, ... }
        let row = sqlx::query(
            r#"
            SELECT id, user_id, name, passkey, created_at, last_used_at, is_active
            FROM passkey_credentials
            WHERE is_active = TRUE
              AND passkey->'cred'->>'cred_id' = $1
            "#,
        )
        .bind(&cred_id_b64)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        row.map(|r| row_to_passkey(&r)).transpose()
    }

    async fn get_passkeys_for_user(&self, user_id: UserId) -> Result<Vec<PasskeyCredential>> {
        let rows = sqlx::query(
            r#"
            SELECT id, user_id, name, passkey, created_at, last_used_at, is_active
            FROM passkey_credentials WHERE user_id = $1
            "#,
        )
        .bind(user_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        rows.iter().map(row_to_passkey).collect()
    }

    async fn update_passkey(&self, credential: &PasskeyCredential) -> Result<()> {
        let passkey_data = serde_json::to_value(&credential.passkey)
            .map_err(|e| AuthError::Repository(e.to_string()))?;

        let result = sqlx::query(
            r#"
            UPDATE passkey_credentials SET
                name = $2, passkey = $3, last_used_at = $4, is_active = $5
            WHERE id = $1
            "#,
        )
        .bind(credential.id.0)
        .bind(&credential.name)
        .bind(&passkey_data)
        .bind(credential.last_used_at)
        .bind(credential.is_active)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        if result.rows_affected() == 0 {
            return Err(AuthError::PasskeyNotFound(credential.id.to_string()));
        }
        Ok(())
    }

    async fn deactivate_passkey(&self, id: PasskeyId) -> Result<()> {
        let result =
            sqlx::query("UPDATE passkey_credentials SET is_active = FALSE WHERE id = $1")
                .bind(id.0)
                .execute(&self.pool)
                .await
                .map_err(sqlx_to_auth_error)?;

        if result.rows_affected() == 0 {
            return Err(AuthError::PasskeyNotFound(id.to_string()));
        }
        Ok(())
    }

    async fn delete_passkey(&self, id: PasskeyId) -> Result<()> {
        sqlx::query("DELETE FROM passkey_credentials WHERE id = $1")
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_auth_error)?;
        Ok(())
    }

    async fn delete_all_passkeys_for_user(&self, user_id: UserId) -> Result<()> {
        sqlx::query("DELETE FROM passkey_credentials WHERE user_id = $1")
            .bind(user_id.0)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_auth_error)?;
        Ok(())
    }

    async fn count_active_passkeys(&self, user_id: UserId) -> Result<u32> {
        let row = sqlx::query(
            "SELECT COUNT(*) as count FROM passkey_credentials WHERE user_id = $1 AND is_active = TRUE",
        )
        .bind(user_id.0)
        .fetch_one(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        let count: i64 = row.get("count");
        Ok(count as u32)
    }
}

fn row_to_passkey(row: &sqlx::postgres::PgRow) -> Result<PasskeyCredential> {
    let passkey_json: serde_json::Value = row.get("passkey");

    Ok(PasskeyCredential {
        id: PasskeyId(row.get("id")),
        user_id: UserId(row.get("user_id")),
        name: row.get("name"),
        passkey: serde_json::from_value(passkey_json)
            .map_err(|e| AuthError::Repository(e.to_string()))?,
        created_at: row.get("created_at"),
        last_used_at: row.get("last_used_at"),
        is_active: row.get("is_active"),
    })
}
