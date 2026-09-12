//! The transactional half of [`crate::store_creation`].
//!
//! Each statement takes a connection rather than `&self`, so the same SQL serves
//! both the standalone trait methods (against a pool connection) and the atomic
//! path (against a transaction) — the same arrangement as
//! [`crate::postgres::invoice_creation`], and for the same reason: two copies of
//! an insert drift apart.

use async_trait::async_trait;
use auth::{Store, StoreRole, UserStore};
use sqlx::PgConnection;

use crate::store_creation::{StoreCreationError, StoreCreationWriter};

use super::PgDataService;
use super::auth::sqlx_to_auth_error;
use super::auth::store::StoreRoleRow;

/// Insert the store row.
pub(super) async fn insert_store(
    conn: &mut PgConnection,
    store: &Store,
) -> Result<(), StoreCreationError> {
    sqlx::query(
        r#"
        INSERT INTO stores (id, name, website, owner_id, archived, created_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(store.id.0)
    .bind(&store.name)
    .bind(&store.website)
    .bind(store.owner_id.0)
    .bind(store.archived)
    .bind(store.created_at)
    .execute(&mut *conn)
    .await
    .map_err(sqlx_to_auth_error)?;
    Ok(())
}

/// Look up a global default role by name.
pub(super) async fn default_role_by_name(
    conn: &mut PgConnection,
    name: &str,
) -> Result<Option<StoreRole>, StoreCreationError> {
    let row = sqlx::query_as::<_, StoreRoleRow>(
        "SELECT id, store_id, role, permissions FROM store_roles \
         WHERE store_id IS NULL AND role = $1",
    )
    .bind(name)
    .fetch_optional(&mut *conn)
    .await
    .map_err(sqlx_to_auth_error)?;

    Ok(row.map(|r| r.try_into()).transpose()?)
}

/// Insert the membership row.
pub(super) async fn insert_user_store(
    conn: &mut PgConnection,
    user_store: &UserStore,
) -> Result<(), StoreCreationError> {
    sqlx::query(
        r#"
        INSERT INTO user_stores (user_id, store_id, store_role_id)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(user_store.user_id.0)
    .bind(user_store.store_id.0)
    .bind(user_store.store_role_id.0)
    .execute(&mut *conn)
    .await
    .map_err(sqlx_to_auth_error)?;
    Ok(())
}

#[async_trait]
impl StoreCreationWriter for PgDataService {
    async fn create_store_owned_by(
        &self,
        store: &Store,
        owner_id: auth::UserId,
    ) -> Result<UserStore, StoreCreationError> {
        let mut tx = self.pool.begin().await.map_err(sqlx_to_auth_error)?;

        insert_store(&mut tx, store).await?;

        // Inside the transaction on purpose. A missing Owner role is the failure
        // that surfaced this bug, and resolving it before opening the transaction
        // would fix only that one ordering while leaving the membership write
        // free to fail with the store already committed.
        let role = default_role_by_name(&mut tx, "Owner")
            .await?
            .ok_or(StoreCreationError::MissingOwnerRole)?;

        let user_store = UserStore::new(owner_id, store.id, role.id);
        insert_user_store(&mut tx, &user_store).await?;

        // Dropping `tx` without this rolls everything back, which is what every
        // `?` above relies on.
        tx.commit().await.map_err(sqlx_to_auth_error)?;
        Ok(user_store)
    }
}
