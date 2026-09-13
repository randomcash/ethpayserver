//! Postgres-backed pending email changes. See `crate::email_change`.

use async_trait::async_trait;
use auth::UserId;
use chrono::{DateTime, Utc};
use sqlx::Row;
use types::{RepositoryError, RepositoryResult};
use uuid::Uuid;

use crate::email_change::{EmailChangeRequest, EmailChangeWriter};
use crate::postgres::PgDataService;

#[async_trait]
impl EmailChangeWriter for PgDataService {
    async fn create_email_change_request(
        &self,
        user_id: UserId,
        new_email: &str,
        expires_at: DateTime<Utc>,
    ) -> RepositoryResult<EmailChangeRequest> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Database(e.to_string()))?;

        // Supersede first: at most one unconsumed request per user must ever
        // exist, so an earlier, possibly wrong, token stops being redeemable
        // the moment a new one is requested.
        sqlx::query("DELETE FROM email_change_requests WHERE user_id = $1 AND consumed_at IS NULL")
            .bind(user_id.0)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Database(e.to_string()))?;

        let row = sqlx::query(
            "INSERT INTO email_change_requests (user_id, new_email, expires_at) \
             VALUES ($1, $2, $3) RETURNING token",
        )
        .bind(user_id.0)
        .bind(new_email)
        .bind(expires_at)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Database(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| RepositoryError::Database(e.to_string()))?;

        Ok(EmailChangeRequest {
            token: row.get("token"),
            user_id,
            new_email: new_email.to_string(),
            expires_at,
        })
    }

    async fn consume_email_change_request(
        &self,
        token: Uuid,
    ) -> RepositoryResult<Option<EmailChangeRequest>> {
        // Atomic take: the WHERE clause is the whole check, so two concurrent
        // confirmations of the same token can only ever have one winner - the
        // loser's UPDATE affects zero rows rather than double-applying.
        let row = sqlx::query(
            "UPDATE email_change_requests SET consumed_at = NOW() \
             WHERE token = $1 AND consumed_at IS NULL AND expires_at > NOW() \
             RETURNING user_id, new_email, expires_at",
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Database(e.to_string()))?;

        Ok(row.map(|row| EmailChangeRequest {
            token,
            user_id: UserId(row.get("user_id")),
            new_email: row.get("new_email"),
            expires_at: row.get("expires_at"),
        }))
    }
}
