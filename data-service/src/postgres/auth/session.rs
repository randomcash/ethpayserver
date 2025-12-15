//! SessionRepository implementation.

use async_trait::async_trait;
use chrono::{Duration, Utc};
use sqlx::Row;

use auth::{DeviceId, Session, SessionId, SessionRepository, UserId, error::{AuthError, Result}};

use super::{PgDataService, sqlx_to_auth_error};

#[async_trait]
impl SessionRepository for PgDataService {
    async fn create_session(&self, session: &Session) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO sessions (
                id, user_id, device_id, created_at, expires_at,
                last_activity_at, ip_address, user_agent
            ) VALUES ($1, $2, $3, $4, $5, $6, $7::inet, $8)
            "#,
        )
        .bind(session.id.0)
        .bind(session.user_id.0)
        .bind(session.device_id.0)
        .bind(session.created_at)
        .bind(session.expires_at)
        .bind(session.last_activity_at)
        .bind(&session.ip_address)
        .bind(&session.user_agent)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(())
    }

    async fn get_session(&self, id: SessionId) -> Result<Option<Session>> {
        let row = sqlx::query(
            r#"
            SELECT id, user_id, device_id, created_at, expires_at,
                   last_activity_at, ip_address::text, user_agent
            FROM sessions WHERE id = $1
            "#,
        )
        .bind(id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(row.map(|r| row_to_session(&r)))
    }

    async fn update_session(&self, session: &Session) -> Result<()> {
        let result = sqlx::query(
            r#"
            UPDATE sessions SET
                last_activity_at = $2, ip_address = $3::inet, user_agent = $4
            WHERE id = $1
            "#,
        )
        .bind(session.id.0)
        .bind(session.last_activity_at)
        .bind(&session.ip_address)
        .bind(&session.user_agent)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        if result.rows_affected() == 0 {
            return Err(AuthError::SessionInvalid);
        }
        Ok(())
    }

    async fn delete_session(&self, id: SessionId) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE id = $1")
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_auth_error)?;
        Ok(())
    }

    async fn delete_all_sessions_for_user(&self, user_id: UserId) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE user_id = $1")
            .bind(user_id.0)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_auth_error)?;
        Ok(())
    }

    async fn delete_sessions_for_device(&self, device_id: DeviceId) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE device_id = $1")
            .bind(device_id.0)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_auth_error)?;
        Ok(())
    }

    async fn delete_expired_sessions(&self) -> Result<u64> {
        let result = sqlx::query("DELETE FROM sessions WHERE expires_at < NOW()")
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_auth_error)?;
        Ok(result.rows_affected())
    }

    async fn delete_stale_sessions(&self, idle_timeout: Option<Duration>) -> Result<u64> {
        let result = match idle_timeout {
            Some(timeout) => {
                let idle_cutoff = Utc::now() - timeout;
                sqlx::query(
                    "DELETE FROM sessions WHERE expires_at < NOW() OR last_activity_at < $1",
                )
                .bind(idle_cutoff)
                .execute(&self.pool)
                .await
                .map_err(sqlx_to_auth_error)?
            }
            None => {
                sqlx::query("DELETE FROM sessions WHERE expires_at < NOW()")
                    .execute(&self.pool)
                    .await
                    .map_err(sqlx_to_auth_error)?
            }
        };
        Ok(result.rows_affected())
    }

    async fn get_sessions_for_user(&self, user_id: UserId) -> Result<Vec<Session>> {
        let rows = sqlx::query(
            r#"
            SELECT id, user_id, device_id, created_at, expires_at,
                   last_activity_at, ip_address::text, user_agent
            FROM sessions WHERE user_id = $1
            "#,
        )
        .bind(user_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(rows.iter().map(row_to_session).collect())
    }
}

fn row_to_session(row: &sqlx::postgres::PgRow) -> Session {
    Session {
        id: SessionId(row.get("id")),
        user_id: UserId(row.get("user_id")),
        device_id: DeviceId(row.get("device_id")),
        created_at: row.get("created_at"),
        expires_at: row.get("expires_at"),
        last_activity_at: row.get("last_activity_at"),
        ip_address: row.get("ip_address"),
        user_agent: row.get("user_agent"),
    }
}
