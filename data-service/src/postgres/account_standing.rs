//! `AccountStandingStore` against `account_standing`.

use async_trait::async_trait;
use sqlx::Row;
use uuid::Uuid;

use crate::account_standing::{AccountStanding, AccountStandingStore, ApplyOutcome, HeldStanding};
use crate::{RepositoryResult, sqlx_to_repo_error};

use super::PgDataService;

#[async_trait]
impl AccountStandingStore for PgDataService {
    async fn apply_account_standing(&self, s: &AccountStanding) -> RepositoryResult<ApplyOutcome> {
        // One statement: Postgres takes the row lock on the conflicting row, so
        // two racing pushes serialise and the loser is judged against the
        // winner's version. No read in application code decides anything.
        let applied = sqlx::query(
            "INSERT INTO account_standing \
                 (account_id, version, in_good_standing, paid_through, plan_name, checkout_url) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (account_id) DO UPDATE SET \
                 version = EXCLUDED.version, \
                 in_good_standing = EXCLUDED.in_good_standing, \
                 paid_through = EXCLUDED.paid_through, \
                 plan_name = EXCLUDED.plan_name, \
                 checkout_url = EXCLUDED.checkout_url, \
                 received_at = now(), \
                 last_heard_at = now() \
             WHERE EXCLUDED.version > account_standing.version \
             RETURNING version",
        )
        .bind(s.account_id)
        .bind(s.version)
        .bind(s.in_good_standing)
        .bind(s.paid_through)
        .bind(&s.plan_name)
        .bind(&s.checkout_url)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        if let Some(row) = applied {
            return Ok(ApplyOutcome::Applied {
                version: row.get("version"),
            });
        }

        // Not applied. Record that the sender was heard only when it repeated
        // the held version; a stale lower one proves nothing about now. Then
        // read the held version, for the response body only.
        sqlx::query(
            "UPDATE account_standing SET last_heard_at = now() \
             WHERE account_id = $1 AND version = $2",
        )
        .bind(s.account_id)
        .bind(s.version)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        let held: i64 =
            sqlx::query_scalar("SELECT version FROM account_standing WHERE account_id = $1")
                .bind(s.account_id)
                .fetch_one(&self.pool)
                .await
                .map_err(sqlx_to_repo_error)?;
        Ok(ApplyOutcome::Kept { held_version: held })
    }

    async fn get_account_standing(
        &self,
        account_id: Uuid,
    ) -> RepositoryResult<Option<HeldStanding>> {
        let row = sqlx::query(
            "SELECT account_id, version, in_good_standing, paid_through, plan_name, \
                    checkout_url, last_heard_at \
             FROM account_standing WHERE account_id = $1",
        )
        .bind(account_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row.map(|r| HeldStanding {
            standing: AccountStanding {
                account_id: r.get("account_id"),
                version: r.get("version"),
                in_good_standing: r.get("in_good_standing"),
                paid_through: r.get("paid_through"),
                plan_name: r.get("plan_name"),
                checkout_url: r.get("checkout_url"),
            },
            last_heard_at: r.get("last_heard_at"),
        }))
    }
}
