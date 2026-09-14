//! WalletRepository implementation.

use async_trait::async_trait;
use sqlx::Row;

use auth::{
    UserId, WalletCredential, WalletCredentialId, WalletRepository,
    error::{AuthError, Result},
};

use super::{PgDataService, sqlx_to_auth_error};

#[async_trait]
impl WalletRepository for PgDataService {
    async fn create_wallet(&self, credential: &WalletCredential) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO wallet_credentials (
                id, user_id, address, name, is_primary, created_at, last_used_at, is_active
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
        )
        .bind(credential.id.0)
        .bind(credential.user_id.0)
        .bind(&credential.address)
        .bind(&credential.name)
        .bind(credential.is_primary)
        .bind(credential.created_at)
        .bind(credential.last_used_at)
        .bind(credential.is_active)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db_err) = e
                && db_err.is_unique_violation()
            {
                return AuthError::WalletAlreadyRegistered;
            }
            sqlx_to_auth_error(e)
        })?;

        Ok(())
    }

    async fn get_wallet(&self, id: WalletCredentialId) -> Result<Option<WalletCredential>> {
        let row = sqlx::query(
            r#"
            SELECT id, user_id, address, name, is_primary, created_at, last_used_at, is_active
            FROM wallet_credentials WHERE id = $1
            "#,
        )
        .bind(id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(row.map(|r| row_to_wallet(&r)))
    }

    async fn get_wallet_by_address(&self, address: &str) -> Result<Option<WalletCredential>> {
        let row = sqlx::query(
            r#"
            SELECT id, user_id, address, name, is_primary, created_at, last_used_at, is_active
            FROM wallet_credentials WHERE address = $1 AND is_active = TRUE
            "#,
        )
        .bind(address)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(row.map(|r| row_to_wallet(&r)))
    }

    async fn get_wallets_for_user(&self, user_id: UserId) -> Result<Vec<WalletCredential>> {
        let rows = sqlx::query(
            r#"
            SELECT id, user_id, address, name, is_primary, created_at, last_used_at, is_active
            FROM wallet_credentials WHERE user_id = $1
            "#,
        )
        .bind(user_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(rows.iter().map(row_to_wallet).collect())
    }

    async fn update_wallet(&self, credential: &WalletCredential) -> Result<()> {
        let result = sqlx::query(
            r#"
            UPDATE wallet_credentials SET
                name = $2, is_primary = $3, last_used_at = $4, is_active = $5
            WHERE id = $1
            "#,
        )
        .bind(credential.id.0)
        .bind(&credential.name)
        .bind(credential.is_primary)
        .bind(credential.last_used_at)
        .bind(credential.is_active)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        if result.rows_affected() == 0 {
            return Err(AuthError::WalletNotFound(credential.id.to_string()));
        }
        Ok(())
    }

    async fn deactivate_wallet(&self, id: WalletCredentialId) -> Result<()> {
        let result = sqlx::query("UPDATE wallet_credentials SET is_active = FALSE WHERE id = $1")
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_auth_error)?;

        if result.rows_affected() == 0 {
            return Err(AuthError::WalletNotFound(id.to_string()));
        }
        Ok(())
    }

    async fn delete_wallet(&self, id: WalletCredentialId) -> Result<()> {
        sqlx::query("DELETE FROM wallet_credentials WHERE id = $1")
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_auth_error)?;
        Ok(())
    }

    async fn delete_all_wallets_for_user(&self, user_id: UserId) -> Result<()> {
        sqlx::query("DELETE FROM wallet_credentials WHERE user_id = $1")
            .bind(user_id.0)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_auth_error)?;
        Ok(())
    }

    async fn count_active_wallets(&self, user_id: UserId) -> Result<u32> {
        let row = sqlx::query(
            "SELECT COUNT(*) as count FROM wallet_credentials WHERE user_id = $1 AND is_active = TRUE",
        )
        .bind(user_id.0)
        .fetch_one(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        let count: i64 = row.get("count");
        Ok(count as u32)
    }
}

impl PgDataService {
    /// Make `wallet_id` the account's primary login wallet, demoting whatever
    /// was primary before it. Sensitive path: this changes an auth credential.
    ///
    /// Not a `WalletRepository` trait method: that trait is defined in the
    /// pinned `auth` crate (payserver-commons), and a proper "set primary"
    /// belongs there long-term, atomic and behind a real DB transaction. This
    /// is the ethpayserver-local equivalent, using only the schema this crate
    /// already owns, so the exactly-one-primary invariant
    /// (`idx_wallet_credentials_one_primary`) can be enforced today rather
    /// than waiting on a commons release.
    ///
    /// `wallet_id` must already be an active `WalletCredential` belonging to
    /// `user_id` - i.e. it must have been through the existing challenge/
    /// signature verification in `complete_wallet_registration` (or account
    /// creation) to get there. That is what proves the caller controls the
    /// address; this method never accepts a bare address plus signature, so
    /// it cannot be used to point `primary_wallet_address` at a key nobody
    /// has verified.
    ///
    /// Demotes the current primary before promoting the new one, in the same
    /// transaction, so `idx_wallet_credentials_one_primary` is never asked to
    /// hold two active primaries - matching the ordering the equivalent
    /// `wallets` (xpub payout) migration documents for the same constraint
    /// shape.
    pub async fn set_primary_wallet_credential(
        &self,
        user_id: UserId,
        wallet_id: WalletCredentialId,
    ) -> Result<WalletCredential> {
        let mut tx = self.pool.begin().await.map_err(sqlx_to_auth_error)?;

        let target = sqlx::query(
            r#"
            SELECT id, user_id, address, name, is_primary, created_at, last_used_at, is_active
            FROM wallet_credentials
            WHERE id = $1 AND user_id = $2 AND is_active = TRUE
            FOR UPDATE
            "#,
        )
        .bind(wallet_id.0)
        .bind(user_id.0)
        .fetch_optional(&mut *tx)
        .await
        .map_err(sqlx_to_auth_error)?
        .map(|r| row_to_wallet(&r))
        .ok_or_else(|| AuthError::WalletNotFound(wallet_id.to_string()))?;

        if target.is_primary {
            tx.commit().await.map_err(sqlx_to_auth_error)?;
            return Ok(target);
        }

        sqlx::query(
            "UPDATE wallet_credentials SET is_primary = FALSE \
             WHERE user_id = $1 AND is_primary = TRUE",
        )
        .bind(user_id.0)
        .execute(&mut *tx)
        .await
        .map_err(sqlx_to_auth_error)?;

        sqlx::query("UPDATE wallet_credentials SET is_primary = TRUE WHERE id = $1")
            .bind(wallet_id.0)
            .execute(&mut *tx)
            .await
            .map_err(sqlx_to_auth_error)?;

        // Keep `users.primary_wallet_address` - the field wallet *login*
        // actually resolves accounts by - in step with the credential that is
        // now primary. `kdf_salt_identifier` is deliberately untouched: it is
        // pinned at registration and this statement does not mention it, so
        // the recovery hash stays valid across the swap.
        sqlx::query("UPDATE users SET primary_wallet_address = $1 WHERE id = $2")
            .bind(&target.address)
            .bind(user_id.0)
            .execute(&mut *tx)
            .await
            .map_err(sqlx_to_auth_error)?;

        tx.commit().await.map_err(sqlx_to_auth_error)?;

        Ok(WalletCredential {
            is_primary: true,
            ..target
        })
    }

    /// Record a fresh proof-of-possession challenge for `address`, replacing
    /// whatever challenge this user had pending.
    ///
    /// One row per user (`ON CONFLICT (user_id) DO UPDATE`), same shape as
    /// the auth crate's own `wallet_challenges`: requesting a second
    /// challenge invalidates the first rather than leaving two live at once.
    /// `created_at` is bound explicitly rather than left to the database's
    /// `NOW()` - the caller embeds this exact timestamp in the message shown
    /// to the wallet extension, and `take_wallet_reauth_challenge` must
    /// return the identical value so the signature it verifies is checked
    /// against the same bytes that were signed.
    pub async fn store_wallet_reauth_challenge(
        &self,
        user_id: UserId,
        address: &str,
        challenge: &str,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> std::result::Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO wallet_reauth_challenges (user_id, address, challenge, created_at)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (user_id) DO UPDATE SET
                address = EXCLUDED.address,
                challenge = EXCLUDED.challenge,
                created_at = EXCLUDED.created_at
            "#,
        )
        .bind(user_id.0)
        .bind(address)
        .bind(challenge)
        .bind(created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Consume the pending wallet-reauth challenge for `user_id`, if any and
    /// if still within its freshness window.
    ///
    /// `DELETE ... RETURNING` in one statement makes this single-use: a
    /// signature answering this challenge cannot be replayed against a
    /// second promotion request.
    pub async fn take_wallet_reauth_challenge(
        &self,
        user_id: UserId,
    ) -> std::result::Result<Option<WalletReauthChallenge>, sqlx::Error> {
        sqlx::query_as::<_, WalletReauthChallenge>(
            r#"
            DELETE FROM wallet_reauth_challenges
            WHERE user_id = $1 AND created_at > NOW() - INTERVAL '5 minutes'
            RETURNING address, challenge, created_at
            "#,
        )
        .bind(user_id.0)
        .fetch_optional(&self.pool)
        .await
    }
}

/// A pending proof-of-possession challenge for promoting a wallet credential
/// to primary. See `store_wallet_reauth_challenge` / `take_wallet_reauth_challenge`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct WalletReauthChallenge {
    pub address: String,
    pub challenge: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

fn row_to_wallet(row: &sqlx::postgres::PgRow) -> WalletCredential {
    WalletCredential {
        id: WalletCredentialId(row.get("id")),
        user_id: UserId(row.get("user_id")),
        address: row.get("address"),
        name: row.get("name"),
        is_primary: row.get("is_primary"),
        created_at: row.get("created_at"),
        last_used_at: row.get("last_used_at"),
        is_active: row.get("is_active"),
    }
}
