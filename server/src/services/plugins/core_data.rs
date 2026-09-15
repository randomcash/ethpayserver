//! Typed read access to core data, for plugins (RCS-257 work item 4).
//!
//! A plugin needs to read core state — the billing plugin needs to know
//! which merchants and stores exist — but cannot be handed a connection to
//! core tables: that would break on any core migration, and we would never
//! be able to change our own schema again. So this goes through typed host
//! calls that return owned data, never raw SQL, the same reasoning as the
//! repository traits in `payserver-commons/types/src/repositories/`.
//!
//! Kept to the smallest set the billing plugin needs, per the ticket: which
//! stores exist, and who owns them. There is no separate "merchants" table —
//! a merchant is a store's `owner_id` — so listing stores already answers
//! both halves of that question.

use async_trait::async_trait;
use data_service::PgDataService;
use types::{StoreId, UserId};

use super::storage::PluginStorageError;

/// A store as visible to a plugin through the host API.
///
/// Deliberately narrower than `types::Store`: no `website`, no
/// `created_at`. Nothing has asked for those yet, and exposing the whole row
/// now would be a surface maintained forever for a plugin that does not
/// exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginStoreSummary {
    pub id: StoreId,
    /// The merchant who owns this store.
    pub owner_id: UserId,
    pub name: String,
    pub archived: bool,
}

/// The host API's typed read surface over core data.
///
/// A plugin is handed a `&dyn PluginCoreDataApi`, never a `PgPool` or a
/// `PgDataService` — this is the entire read surface it gets onto core
/// state.
#[async_trait]
pub trait PluginCoreDataApi: Send + Sync {
    /// Every store, with the merchant who owns it.
    async fn list_stores(&self) -> Result<Vec<PluginStoreSummary>, PluginStorageError>;
}

#[async_trait]
impl PluginCoreDataApi for PgDataService {
    async fn list_stores(&self) -> Result<Vec<PluginStoreSummary>, PluginStorageError> {
        let rows =
            sqlx::query_as::<_, StoreSummaryRow>("SELECT id, owner_id, name, archived FROM stores")
                .fetch_all(self.pool())
                .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }
}

#[derive(sqlx::FromRow)]
struct StoreSummaryRow {
    id: uuid::Uuid,
    owner_id: uuid::Uuid,
    name: String,
    archived: bool,
}

impl From<StoreSummaryRow> for PluginStoreSummary {
    fn from(row: StoreSummaryRow) -> Self {
        Self {
            id: StoreId(row.id),
            owner_id: UserId(row.owner_id),
            name: row.name,
            archived: row.archived,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use sqlx::PgPool;
    use sqlx::postgres::PgPoolOptions;
    use uuid::Uuid;

    use super::*;

    /// `None` only when `DATABASE_URL` is unset — the legitimate "not running
    /// against Postgres" skip. If it's set but the connection fails, that's a
    /// broken test environment, not an absent one: panic instead of
    /// returning `None`, or a bad DB fails the same as no DB at all — a
    /// silent pass instead of the failure it should be.
    async fn test_service() -> Option<PgDataService> {
        let database_url = std::env::var("DATABASE_URL").ok()?;
        let pool = PgPoolOptions::new()
            .max_connections(3)
            .connect(&database_url)
            .await
            .expect("DATABASE_URL is set but connecting to it failed");
        Some(PgDataService::new(pool))
    }

    async fn seed_user(pool: &PgPool) -> Uuid {
        let user_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, kdf_params, encrypted_symmetric_key, \
             recovery_verification_hash, kdf_salt_identifier) \
             VALUES ($1, '{}'::jsonb, '{}'::jsonb, 'h', 'passkey:' || $1::text)",
        )
        .bind(user_id)
        .execute(pool)
        .await
        .unwrap();
        user_id
    }

    async fn seed_store(pool: &PgPool, owner_id: Uuid, name: &str) -> Uuid {
        let store_id = Uuid::new_v4();
        sqlx::query("INSERT INTO stores (id, name, owner_id) VALUES ($1, $2, $3)")
            .bind(store_id)
            .bind(name)
            .bind(owner_id)
            .execute(pool)
            .await
            .unwrap();
        store_id
    }

    /// Ticket item 4: the billing plugin's starting need — every store's id
    /// and owning merchant — comes back as owned, typed data, not a query the
    /// plugin runs itself.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn list_stores_includes_a_known_store_with_its_owner() {
        let Some(service) = test_service().await else {
            return;
        };
        let owner_id = seed_user(service.pool()).await;
        let name = format!("plugin-host-api-test-{}", Uuid::new_v4());
        let store_id = seed_store(service.pool(), owner_id, &name).await;

        let stores = service.list_stores().await.unwrap();

        let found = stores
            .iter()
            .find(|s| s.id.0 == store_id)
            .expect("seeded store should be in the list");
        assert_eq!(found.owner_id.0, owner_id);
        assert_eq!(found.name, name);
        assert!(!found.archived);
    }
}
