//! PostgreSQL implementation of store repositories.

use async_trait::async_trait;

use auth::{
    UserId,
    error::{AuthError, Result},
    repository::{StoreRepository, StoreRoleRepository, UserStoreRepository},
    store::{
        Store, StoreId, StoreInfo, StoreRole, StoreRoleId, StoreRoleInfo, UserStore, UserStoreInfo,
    },
};

use super::{PgDataService, sqlx_to_auth_error};

#[async_trait]
impl StoreRepository for PgDataService {
    async fn create_store(&self, store: &Store) -> Result<()> {
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
        .execute(self.pool())
        .await
        .map_err(sqlx_to_auth_error)?;
        Ok(())
    }

    async fn get_store(&self, id: StoreId) -> Result<Option<Store>> {
        let row = sqlx::query_as::<_, StoreRow>(
            "SELECT id, name, website, owner_id, archived, created_at FROM stores WHERE id = $1",
        )
        .bind(id.0)
        .fetch_optional(self.pool())
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(row.map(|r| r.into()))
    }

    async fn get_stores_for_user(&self, user_id: UserId) -> Result<Vec<Store>> {
        // Get stores where user is owner OR is a member via user_stores
        let rows = sqlx::query_as::<_, StoreRow>(
            r#"
            SELECT DISTINCT s.id, s.name, s.website, s.owner_id, s.archived, s.created_at
            FROM stores s
            LEFT JOIN user_stores us ON s.id = us.store_id
            WHERE s.owner_id = $1 OR us.user_id = $1
            "#,
        )
        .bind(user_id.0)
        .fetch_all(self.pool())
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    async fn get_stores_owned_by(&self, user_id: UserId) -> Result<Vec<Store>> {
        let rows = sqlx::query_as::<_, StoreRow>(
            "SELECT id, name, website, owner_id, archived, created_at FROM stores WHERE owner_id = $1",
        )
        .bind(user_id.0)
        .fetch_all(self.pool())
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    async fn update_store(&self, store: &Store) -> Result<()> {
        let result =
            sqlx::query("UPDATE stores SET name = $2, website = $3, archived = $4 WHERE id = $1")
                .bind(store.id.0)
                .bind(&store.name)
                .bind(&store.website)
                .bind(store.archived)
                .execute(self.pool())
                .await
                .map_err(sqlx_to_auth_error)?;

        if result.rows_affected() == 0 {
            return Err(AuthError::StoreNotFound(store.id.to_string()));
        }
        Ok(())
    }

    async fn archive_store(&self, id: StoreId) -> Result<()> {
        let result = sqlx::query("UPDATE stores SET archived = true WHERE id = $1")
            .bind(id.0)
            .execute(self.pool())
            .await
            .map_err(sqlx_to_auth_error)?;

        if result.rows_affected() == 0 {
            return Err(AuthError::StoreNotFound(id.to_string()));
        }
        Ok(())
    }

    async fn delete_store(&self, id: StoreId) -> Result<()> {
        sqlx::query("DELETE FROM stores WHERE id = $1")
            .bind(id.0)
            .execute(self.pool())
            .await
            .map_err(sqlx_to_auth_error)?;
        Ok(())
    }
}

#[async_trait]
impl StoreRoleRepository for PgDataService {
    async fn create_store_role(&self, role: &StoreRole) -> Result<()> {
        let permissions_json = serde_json::to_value(&role.permissions)
            .map_err(|e| AuthError::Repository(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO store_roles (id, store_id, role, permissions)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(role.id.0)
        .bind(role.store_id.map(|s| s.0))
        .bind(&role.role)
        .bind(permissions_json)
        .execute(self.pool())
        .await
        .map_err(sqlx_to_auth_error)?;
        Ok(())
    }

    async fn get_store_role(&self, id: StoreRoleId) -> Result<Option<StoreRole>> {
        let row = sqlx::query_as::<_, StoreRoleRow>(
            "SELECT id, store_id, role, permissions FROM store_roles WHERE id = $1",
        )
        .bind(id.0)
        .fetch_optional(self.pool())
        .await
        .map_err(sqlx_to_auth_error)?;

        row.map(|r| r.try_into()).transpose()
    }

    async fn get_roles_for_store(&self, store_id: StoreId) -> Result<Vec<StoreRole>> {
        // Get store-specific roles AND global defaults (store_id IS NULL)
        let rows = sqlx::query_as::<_, StoreRoleRow>(
            "SELECT id, store_id, role, permissions FROM store_roles WHERE store_id = $1 OR store_id IS NULL",
        )
        .bind(store_id.0)
        .fetch_all(self.pool())
        .await
        .map_err(sqlx_to_auth_error)?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn get_default_roles(&self) -> Result<Vec<StoreRole>> {
        let rows = sqlx::query_as::<_, StoreRoleRow>(
            "SELECT id, store_id, role, permissions FROM store_roles WHERE store_id IS NULL",
        )
        .fetch_all(self.pool())
        .await
        .map_err(sqlx_to_auth_error)?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn get_default_role_by_name(&self, name: &str) -> Result<Option<StoreRole>> {
        let row = sqlx::query_as::<_, StoreRoleRow>(
            "SELECT id, store_id, role, permissions FROM store_roles WHERE store_id IS NULL AND role = $1",
        )
        .bind(name)
        .fetch_optional(self.pool())
        .await
        .map_err(sqlx_to_auth_error)?;

        row.map(|r| r.try_into()).transpose()
    }

    async fn update_store_role(&self, role: &StoreRole) -> Result<()> {
        let permissions_json = serde_json::to_value(&role.permissions)
            .map_err(|e| AuthError::Repository(e.to_string()))?;

        let result =
            sqlx::query("UPDATE store_roles SET role = $2, permissions = $3 WHERE id = $1")
                .bind(role.id.0)
                .bind(&role.role)
                .bind(permissions_json)
                .execute(self.pool())
                .await
                .map_err(sqlx_to_auth_error)?;

        if result.rows_affected() == 0 {
            return Err(AuthError::StoreRoleNotFound(role.id.to_string()));
        }
        Ok(())
    }

    async fn delete_store_role(&self, id: StoreRoleId) -> Result<()> {
        sqlx::query("DELETE FROM store_roles WHERE id = $1")
            .bind(id.0)
            .execute(self.pool())
            .await
            .map_err(sqlx_to_auth_error)?;
        Ok(())
    }
}

#[async_trait]
impl UserStoreRepository for PgDataService {
    async fn add_user_to_store(&self, user_store: &UserStore) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO user_stores (user_id, store_id, store_role_id)
            VALUES ($1, $2, $3)
            "#,
        )
        .bind(user_store.user_id.0)
        .bind(user_store.store_id.0)
        .bind(user_store.store_role_id.0)
        .execute(self.pool())
        .await
        .map_err(sqlx_to_auth_error)?;
        Ok(())
    }

    async fn get_user_store(
        &self,
        user_id: UserId,
        store_id: StoreId,
    ) -> Result<Option<UserStore>> {
        let row = sqlx::query_as::<_, UserStoreRow>(
            "SELECT user_id, store_id, store_role_id FROM user_stores WHERE user_id = $1 AND store_id = $2",
        )
        .bind(user_id.0)
        .bind(store_id.0)
        .fetch_optional(self.pool())
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(row.map(|r| r.into()))
    }

    async fn get_user_stores(&self, user_id: UserId) -> Result<Vec<UserStore>> {
        let rows = sqlx::query_as::<_, UserStoreRow>(
            "SELECT user_id, store_id, store_role_id FROM user_stores WHERE user_id = $1",
        )
        .bind(user_id.0)
        .fetch_all(self.pool())
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    async fn get_store_users(&self, store_id: StoreId) -> Result<Vec<UserStore>> {
        let rows = sqlx::query_as::<_, UserStoreRow>(
            "SELECT user_id, store_id, store_role_id FROM user_stores WHERE store_id = $1",
        )
        .bind(store_id.0)
        .fetch_all(self.pool())
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    async fn update_user_store(&self, user_store: &UserStore) -> Result<()> {
        let result = sqlx::query(
            "UPDATE user_stores SET store_role_id = $3 WHERE user_id = $1 AND store_id = $2",
        )
        .bind(user_store.user_id.0)
        .bind(user_store.store_id.0)
        .bind(user_store.store_role_id.0)
        .execute(self.pool())
        .await
        .map_err(sqlx_to_auth_error)?;

        if result.rows_affected() == 0 {
            return Err(AuthError::UserNotInStore);
        }
        Ok(())
    }

    async fn remove_user_from_store(&self, user_id: UserId, store_id: StoreId) -> Result<()> {
        sqlx::query("DELETE FROM user_stores WHERE user_id = $1 AND store_id = $2")
            .bind(user_id.0)
            .bind(store_id.0)
            .execute(self.pool())
            .await
            .map_err(sqlx_to_auth_error)?;
        Ok(())
    }

    async fn user_has_store_permission(
        &self,
        user_id: UserId,
        store_id: StoreId,
        permission: &str,
    ) -> Result<bool> {
        // Join user_stores with store_roles and check if permission exists in JSONB array
        let result = sqlx::query_scalar::<_, bool>(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM user_stores us
                JOIN store_roles sr ON us.store_role_id = sr.id
                WHERE us.user_id = $1
                  AND us.store_id = $2
                  AND sr.permissions ? $3
            )
            "#,
        )
        .bind(user_id.0)
        .bind(store_id.0)
        .bind(permission)
        .fetch_one(self.pool())
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(result)
    }

    async fn get_user_store_info(
        &self,
        user_id: UserId,
        store_id: StoreId,
    ) -> Result<Option<UserStoreInfo>> {
        let row = sqlx::query_as::<_, UserStoreInfoRow>(
            r#"
            SELECT
                s.id as store_id, s.name as store_name, s.website as store_website,
                s.archived as store_archived, s.created_at as store_created_at,
                sr.id as role_id, sr.store_id as role_store_id, sr.role as role_name, sr.permissions as role_permissions
            FROM user_stores us
            JOIN stores s ON us.store_id = s.id
            JOIN store_roles sr ON us.store_role_id = sr.id
            WHERE us.user_id = $1 AND us.store_id = $2
            "#,
        )
        .bind(user_id.0)
        .bind(store_id.0)
        .fetch_optional(self.pool())
        .await
        .map_err(sqlx_to_auth_error)?;

        row.map(|r| r.try_into()).transpose()
    }

    async fn get_user_store_infos(&self, user_id: UserId) -> Result<Vec<UserStoreInfo>> {
        let rows = sqlx::query_as::<_, UserStoreInfoRow>(
            r#"
            SELECT
                s.id as store_id, s.name as store_name, s.website as store_website,
                s.archived as store_archived, s.created_at as store_created_at,
                sr.id as role_id, sr.store_id as role_store_id, sr.role as role_name, sr.permissions as role_permissions
            FROM user_stores us
            JOIN stores s ON us.store_id = s.id
            JOIN store_roles sr ON us.store_role_id = sr.id
            WHERE us.user_id = $1
            "#,
        )
        .bind(user_id.0)
        .fetch_all(self.pool())
        .await
        .map_err(sqlx_to_auth_error)?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }
}

// Database row types
#[derive(sqlx::FromRow)]
struct StoreRow {
    id: uuid::Uuid,
    name: String,
    website: Option<String>,
    owner_id: uuid::Uuid,
    archived: bool,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<StoreRow> for Store {
    fn from(row: StoreRow) -> Self {
        Store {
            id: StoreId(row.id),
            name: row.name,
            website: row.website,
            owner_id: UserId(row.owner_id),
            archived: row.archived,
            created_at: row.created_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct StoreRoleRow {
    id: uuid::Uuid,
    store_id: Option<uuid::Uuid>,
    role: String,
    permissions: serde_json::Value,
}

impl TryFrom<StoreRoleRow> for StoreRole {
    type Error = AuthError;

    fn try_from(row: StoreRoleRow) -> std::result::Result<Self, Self::Error> {
        let permissions: Vec<String> = serde_json::from_value(row.permissions)
            .map_err(|e| AuthError::Repository(format!("Failed to parse permissions: {}", e)))?;

        Ok(StoreRole {
            id: StoreRoleId(row.id),
            store_id: row.store_id.map(StoreId),
            role: row.role,
            permissions,
        })
    }
}

#[derive(sqlx::FromRow)]
struct UserStoreRow {
    user_id: uuid::Uuid,
    store_id: uuid::Uuid,
    store_role_id: uuid::Uuid,
}

impl From<UserStoreRow> for UserStore {
    fn from(row: UserStoreRow) -> Self {
        UserStore {
            user_id: UserId(row.user_id),
            store_id: StoreId(row.store_id),
            store_role_id: StoreRoleId(row.store_role_id),
        }
    }
}

#[derive(sqlx::FromRow)]
struct UserStoreInfoRow {
    // Store fields
    store_id: uuid::Uuid,
    store_name: String,
    store_website: Option<String>,
    store_archived: bool,
    store_created_at: chrono::DateTime<chrono::Utc>,
    // Role fields
    role_id: uuid::Uuid,
    role_store_id: Option<uuid::Uuid>,
    role_name: String,
    role_permissions: serde_json::Value,
}

impl TryFrom<UserStoreInfoRow> for UserStoreInfo {
    type Error = AuthError;

    fn try_from(row: UserStoreInfoRow) -> std::result::Result<Self, Self::Error> {
        let permissions: Vec<String> = serde_json::from_value(row.role_permissions)
            .map_err(|e| AuthError::Repository(format!("Failed to parse permissions: {}", e)))?;

        Ok(UserStoreInfo {
            store: StoreInfo {
                id: StoreId(row.store_id),
                name: row.store_name,
                website: row.store_website,
                archived: row.store_archived,
                created_at: row.store_created_at,
            },
            role: StoreRoleInfo {
                id: StoreRoleId(row.role_id),
                store_id: row.role_store_id.map(StoreId),
                role: row.role_name,
                permissions,
            },
        })
    }
}
