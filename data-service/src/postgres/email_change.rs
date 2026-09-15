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
        // Supersede-or-insert in one atomic statement against the unique
        // partial index on (user_id) WHERE consumed_at IS NULL: a separate
        // DELETE-then-INSERT looked equivalent but left a window where two
        // concurrent requests for the same user could each find nothing to
        // delete and both insert, leaving two live tokens. ON CONFLICT closes
        // that window - only one row can ever exist for this user with
        // consumed_at IS NULL, and this statement either creates it or
        // replaces it, never both. `token` is reassigned on conflict too, so
        // the superseded request's token stops being the one in flight.
        let row = sqlx::query(
            "INSERT INTO email_change_requests (user_id, new_email, expires_at) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (user_id) WHERE consumed_at IS NULL \
             DO UPDATE SET new_email = EXCLUDED.new_email, \
                 expires_at = EXCLUDED.expires_at, \
                 created_at = NOW(), \
                 token = uuid_generate_v4() \
             RETURNING token",
        )
        .bind(user_id.0)
        .bind(new_email)
        .bind(expires_at)
        .fetch_one(&self.pool)
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
