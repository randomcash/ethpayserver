//! One connection pool per plugin, and a single budget shared by all of them.
//!
//! A plugin's statements run on a connection Postgres has authenticated as
//! *that plugin's* role, which is the only way the grants in
//! [`PluginStorage::provision_role`](super::PluginStorage::provision_role)
//! mean anything. A `SET ROLE` on the host's connection would not do: a
//! session can always `RESET ROLE` back to the role it authenticated as, so
//! a plugin supplying its own SQL would simply step back out.
//!
//! # Why this scales to any number of plugins
//!
//! Roles and schemas are cheap - rows in `pg_authid` and `pg_namespace`, and
//! thousands are unremarkable. **Connections are the scarce thing**: Postgres
//! defaults to 100 total, shared with the core pool, and each one costs real
//! memory on the server.
//!
//! So a fixed pool per plugin does not scale, and the design does not use one:
//!
//! - Pools are **lazy and idle to zero**. A `PgPool` is a configuration
//!   object, not a connection; with `min_connections(0)` and a short idle
//!   timeout, a plugin that is not being called holds nothing. A thousand
//!   installed plugins that are all quiet cost a thousand structs and zero
//!   connections.
//! - A **global semaphore** bounds how much plugin database work is in flight
//!   at once, across every plugin. That is the limit that actually holds the
//!   line, and it does not move when the plugin count does. Past it, callers
//!   queue rather than opening connections.
//!
//! The per-role `CONNECTION LIMIT` set at provisioning is the third layer: it
//! stops one plugin monopolising the shared budget even while it holds
//! permits.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use payserver_plugin_api::PluginId;
use sqlx::Transaction;
use sqlx::postgres::{PgPool, PgPoolOptions, Postgres};
use tokio::sync::{OwnedSemaphorePermit, RwLock, Semaphore};

/// Connections one plugin's pool may open. Matches the `CONNECTION LIMIT`
/// set on the role itself, so the pool never promises what the database will
/// refuse.
const POOL_MAX_CONNECTIONS: u32 = 2;

/// How long an unused connection is kept before being closed. Short, because
/// the point is that a quiet plugin holds nothing; long enough that a burst
/// of calls does not reconnect for each one.
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a caller waits for a connection from its own pool before giving
/// up. Bounded so a plugin whose own two connections are busy fails rather
/// than blocking a host thread indefinitely.
const POOL_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a caller waits for a slot in the global budget. Deliberately
/// short: a queue this long means the instance is saturated, and a plugin
/// that is told so can fail open rather than stall the request behind it.
const PERMIT_TIMEOUT: Duration = Duration::from_secs(5);

/// How much plugin database work may be in flight at once, across every
/// plugin on the instance.
pub const DEFAULT_MAX_IN_FLIGHT: usize = 16;

/// Why a plugin could not get at its database.
#[derive(Debug, thiserror::Error)]
pub enum PluginPoolError {
    /// No pool is registered for this plugin - it was never installed with a
    /// role, or its pool was removed when it was disabled.
    #[error("plugin {0} has no database access")]
    NotRegistered(PluginId),

    /// The instance-wide budget is exhausted. Distinct from a connection
    /// failure: the database is fine, we are simply busy, and a caller may
    /// reasonably treat the two differently.
    #[error("the instance is at its limit for plugin database work")]
    Busy,

    /// `DATABASE_URL` could not be rewritten to authenticate as a role.
    #[error("could not build a connection string for a plugin role: {0}")]
    BadDatabaseUrl(String),

    #[error("database error: {0}")]
    Database(String),
}

/// Every plugin's pool, plus the budget they share.
pub struct PluginPools {
    /// The host's own `DATABASE_URL`, used only for its host, port, database
    /// and options - the credentials in it are replaced per plugin.
    base_url: String,
    pools: RwLock<HashMap<PluginId, PgPool>>,
    permits: Arc<Semaphore>,
}

impl PluginPools {
    #[must_use]
    pub fn new(base_url: String, max_in_flight: usize) -> Self {
        Self {
            base_url,
            pools: RwLock::new(HashMap::new()),
            // `max(1)` because a zero-permit semaphore is not a tighter
            // limit, it is a deadlock: every acquire would wait forever for a
            // permit that can never be issued.
            permits: Arc::new(Semaphore::new(max_in_flight.max(1))),
        }
    }

    /// Register `id`'s pool, authenticating as its role.
    ///
    /// Lazy: this opens no connection. A plugin that is installed and never
    /// called costs nothing, which is what lets the plugin count be
    /// unbounded. The cost is that a wrong password is not discovered here -
    /// the install path verifies once, eagerly, so that a broken credential
    /// fails the install rather than surfacing later as a mysterious runtime
    /// error.
    ///
    /// # Errors
    /// If `base_url` cannot be rewritten for this role.
    pub async fn register(&self, id: &PluginId, password: &str) -> Result<(), PluginPoolError> {
        let url = connection_url_for(&self.base_url, id.as_str(), password)?;
        let pool = PgPoolOptions::new()
            .max_connections(POOL_MAX_CONNECTIONS)
            .min_connections(0)
            .idle_timeout(Some(POOL_IDLE_TIMEOUT))
            .acquire_timeout(POOL_ACQUIRE_TIMEOUT)
            .connect_lazy(&url)
            .map_err(|e| PluginPoolError::BadDatabaseUrl(e.to_string()))?;
        self.pools.write().await.insert(id.clone(), pool);
        Ok(())
    }

    /// Close and forget `id`'s pool.
    ///
    /// Must happen before the role is dropped: Postgres refuses to drop a
    /// role while a session is authenticated as it, so an uninstall that
    /// skipped this would fail half-way and leave the role behind.
    pub async fn remove(&self, id: &PluginId) {
        let pool = self.pools.write().await.remove(id);
        if let Some(pool) = pool {
            pool.close().await;
        }
    }

    /// Whether `id` has database access registered.
    pub async fn is_registered(&self, id: &PluginId) -> bool {
        self.pools.read().await.contains_key(id)
    }

    /// Begin a transaction for `id`, holding a slot in the global budget for
    /// as long as it lives.
    ///
    /// The permit is returned alongside the transaction rather than dropped
    /// here on purpose: releasing it at the end of this function would bound
    /// how many transactions *start* at once and not how many are open, which
    /// is the number that actually costs connections.
    ///
    /// # Errors
    /// [`PluginPoolError::NotRegistered`] if the plugin has no pool,
    /// [`PluginPoolError::Busy`] if the instance-wide budget is exhausted,
    /// and [`PluginPoolError::Database`] if the connection itself fails.
    pub async fn begin(
        &self,
        id: &PluginId,
    ) -> Result<(OwnedSemaphorePermit, Transaction<'static, Postgres>), PluginPoolError> {
        let pool = {
            let pools = self.pools.read().await;
            pools
                .get(id)
                .cloned()
                .ok_or_else(|| PluginPoolError::NotRegistered(id.clone()))?
        };

        let permit =
            tokio::time::timeout(PERMIT_TIMEOUT, Arc::clone(&self.permits).acquire_owned())
                .await
                .map_err(|_| PluginPoolError::Busy)?
                .map_err(|_| PluginPoolError::Busy)?;

        let tx = pool
            .begin()
            .await
            .map_err(|e| PluginPoolError::Database(e.to_string()))?;
        Ok((permit, tx))
    }
}

/// Rewrite `base_url` to authenticate as `role`.
///
/// Only the credentials change: host, port, database and any query options
/// (`sslmode`, and so on) are carried across untouched, so a plugin connects
/// to exactly the database the host does, on the same terms.
///
/// The role name is percent-encoded. Plugin schema names contain dots, which
/// are safe in a URL's userinfo, but the name is attacker-adjacent data
/// reaching a parser and encoding it costs nothing.
fn connection_url_for(
    base_url: &str,
    plugin_id: &str,
    password: &str,
) -> Result<String, PluginPoolError> {
    let (scheme, rest) = base_url
        .split_once("://")
        .ok_or_else(|| PluginPoolError::BadDatabaseUrl("no scheme".to_string()))?;

    // Split credentials off the front, if any. `rsplit_once` rather than
    // `split_once`: a password may itself contain '@', and the last one is
    // the separator.
    let host_and_rest = rest.rsplit_once('@').map_or(rest, |(_, after)| after);
    if host_and_rest.is_empty() {
        return Err(PluginPoolError::BadDatabaseUrl(
            "no host after credentials".to_string(),
        ));
    }

    // The role and the schema share a name, so this is the same identifier
    // `provision_role` granted against. Deriving it twice from the same
    // function is what keeps a connection from ever authenticating as a role
    // whose grants point somewhere else.
    let role = super::storage::role_name(plugin_id)
        .map_err(|e| PluginPoolError::BadDatabaseUrl(e.to_string()))?;

    Ok(format!(
        "{scheme}://{}:{}@{host_and_rest}",
        percent_encode(&role),
        percent_encode(password)
    ))
}

/// Percent-encodes everything outside the URL unreserved set.
fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn id(s: &str) -> PluginId {
        PluginId::new(s).unwrap()
    }

    #[test]
    fn a_role_url_keeps_the_host_database_and_options() {
        let url = connection_url_for(
            "postgres://ethpayserver:hunter2@db.internal:5432/ethpayserver?sslmode=require",
            "cash.random.billing",
            "abc123",
        )
        .unwrap();

        assert!(
            url.ends_with("@db.internal:5432/ethpayserver?sslmode=require"),
            "host, database and options must survive: {url}"
        );
        assert!(
            url.starts_with("postgres://plugin_cash.random.billing:abc123@"),
            "credentials must be the plugin's: {url}"
        );
        assert!(
            !url.contains("hunter2"),
            "the host's own password must not survive into a plugin's url: {url}"
        );
    }

    /// A password containing '@' is legal and splitting on the *first* one
    /// would take part of the password for the host.
    #[test]
    fn a_host_password_containing_an_at_sign_does_not_confuse_the_split() {
        let url = connection_url_for(
            "postgres://user:p@ss@db.internal:5432/core",
            "cash.random.billing",
            "abc123",
        )
        .unwrap();
        assert!(url.ends_with("@db.internal:5432/core"), "{url}");
    }

    #[test]
    fn a_url_without_credentials_still_works() {
        let url =
            connection_url_for("postgres://localhost/core", "cash.random.billing", "abc").unwrap();
        assert_eq!(
            url,
            "postgres://plugin_cash.random.billing:abc@localhost/core"
        );
    }

    #[test]
    fn a_url_without_a_scheme_is_refused() {
        assert!(matches!(
            connection_url_for("localhost/core", "cash.random.billing", "abc"),
            Err(PluginPoolError::BadDatabaseUrl(_))
        ));
    }

    #[tokio::test]
    async fn an_unregistered_plugin_has_no_database_access() {
        let pools = PluginPools::new("postgres://localhost/core".to_string(), 4);
        let result = pools.begin(&id("cash.random.ghost")).await;
        assert!(matches!(result, Err(PluginPoolError::NotRegistered(_))));
        assert!(!pools.is_registered(&id("cash.random.ghost")).await);
    }

    /// The budget is what makes the plugin count irrelevant, so it has to
    /// actually hold. Two permits, two transactions' worth of work in
    /// flight, and the third caller is told the instance is busy rather than
    /// opening a third connection.
    #[tokio::test]
    async fn the_global_budget_is_shared_across_plugins() {
        let pools = PluginPools::new("postgres://localhost/core".to_string(), 2);
        let held: Vec<_> = vec![
            Arc::clone(&pools.permits).try_acquire_owned().unwrap(),
            Arc::clone(&pools.permits).try_acquire_owned().unwrap(),
        ];

        assert!(
            Arc::clone(&pools.permits).try_acquire_owned().is_err(),
            "a third slot was issued against a budget of two"
        );

        drop(held);
        assert!(
            Arc::clone(&pools.permits).try_acquire_owned().is_ok(),
            "slots must return to the budget when work finishes"
        );
    }

    /// A zero budget would be a deadlock rather than a stricter limit: every
    /// caller would wait for a permit that can never be issued.
    #[tokio::test]
    async fn a_zero_budget_is_treated_as_one_rather_than_deadlocking() {
        let pools = PluginPools::new("postgres://localhost/core".to_string(), 0);
        assert!(Arc::clone(&pools.permits).try_acquire_owned().is_ok());
    }

    #[tokio::test]
    async fn removing_a_pool_that_was_never_registered_is_fine() {
        let pools = PluginPools::new("postgres://localhost/core".to_string(), 4);
        pools.remove(&id("cash.random.absent")).await;
    }

    /// End to end against real Postgres: provision a role, register its pool,
    /// and confirm the connection the pool hands out is authenticated as that
    /// role and bounded by its grants.
    ///
    /// This is the join between the two halves. `storage`'s tests prove the
    /// grants are right by building their own connection; these prove the
    /// pool actually uses them, which is the part that would silently fall
    /// back to the host's superuser connection if the URL rewriting were
    /// wrong.
    #[tokio::test]
    #[ignore]
    async fn a_pool_connects_as_the_plugin_role_and_is_bounded_by_its_grants() {
        use crate::services::plugins::{PluginStorage, generate_role_password};

        let Ok(url) = std::env::var("DATABASE_URL") else {
            return;
        };
        let host_pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("DATABASE_URL is set but connecting failed");
        let storage = PluginStorage::new(host_pool.clone());
        let plugin = id("cash.random.poolscope");

        // Whatever a previous failed run left behind.
        let _ = storage.drop_role(&plugin).await;
        let _ = storage.uninstall(&plugin, true).await;

        let migrations = tempfile::tempdir().unwrap();
        storage.install(&plugin, migrations.path()).await.unwrap();
        let password = generate_role_password().unwrap();
        storage.provision_role(&plugin, &password).await.unwrap();

        let schema = crate::services::plugins::role_name(plugin.as_str()).unwrap();
        sqlx::query(&format!("CREATE TABLE \"{schema}\".notes (body text)"))
            .execute(&host_pool)
            .await
            .unwrap();

        let pools = PluginPools::new(url, 4);
        pools.register(&plugin, &password).await.unwrap();

        // Its own table, through the pool.
        let (permit, mut tx) = pools.begin(&plugin).await.unwrap();
        sqlx::query(&format!("INSERT INTO \"{schema}\".notes VALUES ('hello')"))
            .execute(&mut *tx)
            .await
            .expect("a plugin must reach its own schema through its pool");
        tx.commit().await.unwrap();
        drop(permit);

        // Who the pool actually authenticated as.
        let (permit, mut tx) = pools.begin(&plugin).await.unwrap();
        let who: (String,) = sqlx::query_as("SELECT current_user")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        assert_eq!(
            who.0, schema,
            "the pool connected as the wrong role; grants would not apply"
        );

        // And still cannot reach core.
        let denied = sqlx::query("SELECT id FROM public.installed_plugins LIMIT 1")
            .fetch_optional(&mut *tx)
            .await;
        assert!(
            denied.is_err(),
            "a plugin reached a core table through its own pool"
        );
        drop(tx);
        drop(permit);

        pools.remove(&plugin).await;
        storage.drop_role(&plugin).await.unwrap();
        storage.uninstall(&plugin, true).await.unwrap();
    }

    /// `DROP ROLE` fails while a session is authenticated as the role, so an
    /// uninstall that dropped the role without closing the pool first would
    /// fail half-way and leave the role behind. This pins the ordering.
    #[tokio::test]
    #[ignore]
    async fn the_role_can_be_dropped_once_its_pool_is_closed() {
        use crate::services::plugins::{PluginStorage, generate_role_password};

        let Ok(url) = std::env::var("DATABASE_URL") else {
            return;
        };
        let host_pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("connect");
        let storage = PluginStorage::new(host_pool);
        let plugin = id("cash.random.pooldrop");

        let _ = storage.drop_role(&plugin).await;
        let _ = storage.uninstall(&plugin, true).await;

        let migrations = tempfile::tempdir().unwrap();
        storage.install(&plugin, migrations.path()).await.unwrap();
        let password = generate_role_password().unwrap();
        storage.provision_role(&plugin, &password).await.unwrap();

        let pools = PluginPools::new(url, 4);
        pools.register(&plugin, &password).await.unwrap();

        // Open a real connection so the role is genuinely in use.
        let (permit, mut tx) = pools.begin(&plugin).await.unwrap();
        sqlx::query("SELECT 1").execute(&mut *tx).await.unwrap();
        drop(tx);
        drop(permit);

        pools.remove(&plugin).await;
        storage
            .drop_role(&plugin)
            .await
            .expect("the role must be droppable once its pool is closed");
        storage.uninstall(&plugin, true).await.unwrap();
    }
}
