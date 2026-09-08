//! Wallet rotation repository implementation.

use sqlx::Row;
use uuid::Uuid;

use super::PgDataService;
use crate::{RepositoryResult, sqlx_to_repo_error};

/// A recorded wallet rotation event.
#[derive(Debug, Clone)]
pub struct WalletRotation {
    pub id: Uuid,
    pub store_id: Uuid,
    pub previous_xpub: String,
    pub new_xpub: String,
    pub payment_method_id: Uuid,
    pub previous_derivation_index: i32,
    pub reason: Option<String>,
    pub rotated_at: chrono::DateTime<chrono::Utc>,
}

fn row_to_rotation(row: &sqlx::postgres::PgRow) -> WalletRotation {
    WalletRotation {
        id: row.get("id"),
        store_id: row.get("store_id"),
        previous_xpub: row.get("previous_xpub"),
        new_xpub: row.get("new_xpub"),
        payment_method_id: row.get("payment_method_id"),
        previous_derivation_index: row.get("previous_derivation_index"),
        reason: row.get("reason"),
        rotated_at: row.get("rotated_at"),
    }
}

impl PgDataService {
    /// Record a wallet rotation and repoint the payment method at the new key.
    ///
    /// Atomically:
    /// 1. Inserts a rotation record preserving the old xpub and its index
    /// 2. Points the payment method at the account wallet holding the new xpub
    ///
    /// Since RCS-234 this repoints rather than overwrites. It used to write
    /// the new xpub onto the payment method and set `derivation_index = 0`;
    /// that reset was safe only because the row owned its counter outright.
    /// A wallet is shared, so zeroing it would re-issue every address the key
    /// had already produced for every other method using it. The new wallet
    /// arrives knowing its own position instead - 0 if the key is new to the
    /// account, and wherever it had got to if it is not.
    ///
    /// Returns the rotation record.
    pub async fn rotate_payment_method_xpub(
        &self,
        store_id: Uuid,
        payment_method_id: Uuid,
        new_xpub: &str,
        reason: Option<&str>,
    ) -> RepositoryResult<WalletRotation> {
        let mut tx = self.pool.begin().await.map_err(sqlx_to_repo_error)?;

        // Fetch current state through the wallet the method points at, and
        // lock that wallet: the index recorded below has to be the one no
        // further address was issued past, so nothing may allocate between
        // reading it and the repoint.
        let current = sqlx::query(
            r#"
            SELECT w.id AS wallet_id, w.xpub, w.derivation_index
            FROM store_payment_methods pm
            JOIN wallets w ON w.id = pm.wallet_id
            WHERE pm.id = $1 AND pm.store_id = $2
            FOR UPDATE OF w
            "#,
        )
        .bind(payment_method_id)
        .bind(store_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(sqlx_to_repo_error)?;

        let current = current
            .ok_or_else(|| crate::RepositoryError::NotFound("payment method not found".into()))?;

        let previous_xpub: String = current.get("xpub");
        let previous_derivation_index: i32 = current.get("derivation_index");

        // Find or create the account wallet for the new key. Same find-or-
        // create as configuring a method: rotating two methods onto one new
        // xpub must land them on one wallet, not two counters.
        let new_wallet = sqlx::query(
            r#"
            INSERT INTO wallets (user_id, xpub, is_primary)
            SELECT s.owner_id, $2, NOT EXISTS (
                SELECT 1 FROM wallets WHERE user_id = s.owner_id
            )
            FROM stores s WHERE s.id = $1
            ON CONFLICT (user_id, xpub) DO UPDATE SET xpub = EXCLUDED.xpub
            RETURNING id
            "#,
        )
        .bind(store_id)
        .bind(new_xpub)
        .fetch_optional(&mut *tx)
        .await
        .map_err(sqlx_to_repo_error)?
        .ok_or_else(|| crate::RepositoryError::NotFound("store not found".into()))?;

        let new_wallet_id: Uuid = new_wallet.get("id");

        // Insert rotation record
        let rotation_row = sqlx::query(
            r#"
            INSERT INTO wallet_rotations
                (store_id, previous_xpub, new_xpub, payment_method_id, previous_derivation_index, reason)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, store_id, previous_xpub, new_xpub, payment_method_id, previous_derivation_index, reason, rotated_at
            "#,
        )
        .bind(store_id)
        .bind(&previous_xpub)
        .bind(new_xpub)
        .bind(payment_method_id)
        .bind(previous_derivation_index)
        .bind(reason)
        .fetch_one(&mut *tx)
        .await
        .map_err(sqlx_to_repo_error)?;

        // Repoint the method. No counter is touched: the destination wallet
        // already holds the only correct position for its own key.
        sqlx::query(
            r#"
            UPDATE store_payment_methods
            SET wallet_id = $1
            WHERE id = $2
            "#,
        )
        .bind(new_wallet_id)
        .bind(payment_method_id)
        .execute(&mut *tx)
        .await
        .map_err(sqlx_to_repo_error)?;

        tx.commit().await.map_err(sqlx_to_repo_error)?;

        Ok(row_to_rotation(&rotation_row))
    }

    /// Get rotation history for a store.
    pub async fn get_rotation_history(
        &self,
        store_id: Uuid,
    ) -> RepositoryResult<Vec<WalletRotation>> {
        let rows = sqlx::query(
            r#"
            SELECT id, store_id, previous_xpub, new_xpub, payment_method_id, previous_derivation_index, reason, rotated_at
            FROM wallet_rotations
            WHERE store_id = $1
            ORDER BY rotated_at DESC
            "#,
        )
        .bind(store_id)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(rows.iter().map(row_to_rotation).collect())
    }
}
