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
    /// Record a wallet rotation and update the payment method's xpub.
    ///
    /// Atomically:
    /// 1. Inserts a rotation record preserving the old xpub
    /// 2. Updates the payment method to the new xpub with derivation_index reset to 0
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

        // Fetch current state of the payment method
        let current = sqlx::query(
            r#"
            SELECT xpub, derivation_index
            FROM store_payment_methods
            WHERE id = $1 AND store_id = $2
            FOR UPDATE
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

        // Update the payment method to use the new xpub, reset derivation index
        sqlx::query(
            r#"
            UPDATE store_payment_methods
            SET xpub = $1, derivation_index = 0
            WHERE id = $2
            "#,
        )
        .bind(new_xpub)
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
