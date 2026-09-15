//! Schema-per-plugin storage: creation, migrations, and a scoped connection
//! handle.
//!
//! Copies BTCPay's `BaseDbContextFactory<T>` shape: each plugin gets its own
//! Postgres schema and ships its own migrations, which the host runs on
//! install and upgrade. A plugin never gets a connection to `public` — see
//! [`PluginSchema::acquire`] for what that scoping does and does not buy.
//!
//! ## For plugin authors: migration checksums are baked in at compile time
//!
//! `sqlx` records a SHA-384 of each migration file's *bytes* the first time
//! it applies, and refuses to run again if that checksum ever changes — this
//! is the same trap documented for the host's own migrations in this repo's
//! `CLAUDE.md`, and it applies identically to a plugin's own migrations
//! directory. Once a plugin build has shipped a migration and it has run
//! against even one installation, editing that file — including something
//! as small as fixing a comment or reformatting whitespace — breaks every
//! future install or upgrade against a database where it already applied.
//! Add a new migration instead of editing an old one; renaming an
//! unapplied file is free, changing its bytes is not.

use std::path::Path;

use payserver_plugin_api::PluginId;
use sqlx::Transaction;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgPool, Postgres};

/// Postgres's identifier length limit (`NAMEDATALEN` 64, minus the
/// terminator). A schema name past this is silently truncated by Postgres,
/// which could make two differently-named plugins collide on one schema —
/// refused up front instead of discovered later as data corruption.
const MAX_IDENTIFIER_LEN: usize = 63;

/// Why a plugin storage operation failed.
#[derive(Debug, thiserror::Error)]
pub enum PluginStorageError {
    /// `plugin_<id>` is longer than Postgres identifiers allow.
    #[error(
        "plugin id {0:?} produces a schema name longer than {MAX_IDENTIFIER_LEN} bytes; \
         Postgres would silently truncate it"
    )]
    SchemaNameTooLong(String),

    #[error("plugin storage database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("plugin migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
}

/// The Postgres schema name for `id`: `plugin_<id>`. The prefix keeps it out
/// of the way of `public` and of any core schema that might exist later.
fn schema_name(id: &PluginId) -> Result<String, PluginStorageError> {
    let name = format!("plugin_{}", id.as_str());
    if name.len() > MAX_IDENTIFIER_LEN {
        return Err(PluginStorageError::SchemaNameTooLong(name));
    }
    Ok(name)
}

/// Double-quote a Postgres identifier, escaping embedded quotes.
///
/// Every caller here builds `name` from [`schema_name`], which in turn
/// builds it from a [`PluginId`] — already restricted to ASCII
/// alphanumerics, `.`, `-` and `_` by [`PluginId::new`], so there is never
/// actually a quote to escape. Escaping anyway costs nothing and keeps this
/// correct if that charset ever loosens.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Schema-per-plugin storage, backed by the same database as core data.
#[derive(Clone)]
pub struct PluginStorage {
    pool: PgPool,
}

impl PluginStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create `id`'s schema if it does not already exist, then run the
    /// migrations at `migrations_dir` against it.
    ///
    /// Serves both install and upgrade: schema creation is idempotent, and
    /// sqlx tracks applied versions inside the plugin's own schema, so
    /// calling this again after a fixed migration resumes rather than
    /// reapplying what already succeeded.
    ///
    /// A failed migration returns `Err`. Whatever migrations already
    /// committed before the failing one stay applied — each runs in its own
    /// transaction — but this method does not report success, and the
    /// caller must not register the plugin as installed. That is what keeps
    /// a failed migration from leaving the plugin half-installed instead of
    /// not installed.
    pub async fn install(
        &self,
        id: &PluginId,
        migrations_dir: &Path,
    ) -> Result<PluginSchema, PluginStorageError> {
        let name = schema_name(id)?;

        sqlx::query(&format!(
            "CREATE SCHEMA IF NOT EXISTS {}",
            quote_ident(&name)
        ))
        .execute(&self.pool)
        .await?;

        let migrator = Migrator::new(migrations_dir).await?;
        let mut conn = self.pool.acquire().await?;
        sqlx::query(&format!("SET search_path TO {}", quote_ident(&name)))
            .execute(&mut *conn)
            .await?;
        let migration_result = migrator.run(&mut *conn).await;

        // `conn` is a physical connection borrowed from the shared pool, and
        // `SET` (unlike `SET LOCAL`) changes it for the rest of the
        // connection's life, not just this borrow — sqlx does not reset
        // session state on release. Left alone, the next unrelated query
        // that happens to reuse this connection (including a core query
        // that assumes the default `public` search_path) would run scoped
        // to this plugin's schema instead. Reset unconditionally, whether
        // the migration succeeded or not, before deciding what to return.
        let reset_result = sqlx::query("RESET search_path").execute(&mut *conn).await;
        migration_result?;
        reset_result?;

        Ok(PluginSchema {
            pool: self.pool.clone(),
            name,
        })
    }

    /// Uninstall `id`. Dropping the schema is opt-in via `drop_schema`
    /// because an admin who uninstalls a plugin to debug it
    /// should not lose its data — the billing plugin's subscription records
    /// most of all. When `drop_schema` is `false` this leaves the schema and
    /// its data in place for a future reinstall to resume against.
    pub async fn uninstall(
        &self,
        id: &PluginId,
        drop_schema: bool,
    ) -> Result<(), PluginStorageError> {
        if !drop_schema {
            return Ok(());
        }
        let name = schema_name(id)?;
        sqlx::query(&format!(
            "DROP SCHEMA IF EXISTS {} CASCADE",
            quote_ident(&name)
        ))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// A handle to `id`'s own schema for the host API. Does not require `id`
    /// to have just been installed — a caller that already knows a plugin is
    /// installed can go straight here without redoing `install`.
    pub fn schema(&self, id: &PluginId) -> Result<PluginSchema, PluginStorageError> {
        Ok(PluginSchema {
            pool: self.pool.clone(),
            name: schema_name(id)?,
        })
    }
}

/// A handle scoped to one plugin's own schema: the host API's item 3, a
/// plugin can reach its own tables through here and nothing else.
#[derive(Clone)]
pub struct PluginSchema {
    pool: PgPool,
    name: String,
}

impl PluginSchema {
    /// The schema's unquoted name, e.g. `plugin_cash.random.billing`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// A transaction whose `search_path` is this schema alone (no
    /// `public`), so an unqualified table name in a query the plugin issues
    /// can only resolve inside its own schema. The caller must `commit` it
    /// for any writes to persist; dropping it without committing rolls it
    /// back, same as any other sqlx transaction.
    ///
    /// This hands back a transaction rather than a bare pooled connection
    /// so the scoping can use `SET LOCAL` instead of `SET`: `SET LOCAL`
    /// reverts automatically when the transaction ends, by commit or by
    /// drop-triggered rollback, whereas plain `SET` changes the physical
    /// connection for the rest of its life. Since connections here are
    /// borrowed from the same pool the host's own core queries draw from,
    /// a `SET` that outlived this call would leak this plugin's search
    /// path into whatever unrelated query reuses the connection next.
    ///
    /// This is a default, not an enforcement boundary: a query that
    /// qualifies a table explicitly (`public.invoices`) still reaches it,
    /// because the connection runs as the same database role as the rest of
    /// the host. Real enforcement would mean a per-plugin database role with
    /// grants revoked on every other schema, which is out of scope here and
    /// unnecessary for the threat this actually defends against: the admin
    /// installing a plugin already trusts it. What
    /// schema-scoping buys is blast radius and upgradability: a plugin that
    /// only ever names its own tables cannot be broken by a core migration,
    /// and core migrations stay free to change core tables however they
    /// need to.
    pub async fn acquire(&self) -> Result<Transaction<'static, Postgres>, PluginStorageError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(&format!(
            "SET LOCAL search_path TO {}",
            quote_ident(&self.name)
        ))
        .execute(&mut *tx)
        .await?;
        Ok(tx)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use sqlx::Row;
    use sqlx::postgres::PgPoolOptions;

    use super::*;

    fn plugin_id(id: &str) -> PluginId {
        PluginId::new(id).unwrap()
    }

    /// `None` only when `DATABASE_URL` is unset — the legitimate "not running
    /// against Postgres" skip. If it's set but the connection fails, that's a
    /// broken test environment, not an absent one: panic instead of
    /// returning `None`, or a bad DB fails the same as no DB at all — a
    /// silent pass instead of the failure it should be.
    async fn test_pool() -> Option<PgPool> {
        let database_url = std::env::var("DATABASE_URL").ok()?;
        Some(
            PgPoolOptions::new()
                .max_connections(3)
                .connect(&database_url)
                .await
                .expect("DATABASE_URL is set but connecting to it failed"),
        )
    }

    /// A fresh migrations directory with one migration that creates a table,
    /// so a test can install a plugin without shipping a fixture on disk.
    fn write_migration(dir: &Path, version: &str, sql: &str) {
        std::fs::write(dir.join(format!("{version}_test.sql")), sql).unwrap();
    }

    /// Ticket item 1 + 2: install creates the plugin's own schema and runs
    /// its migrations there, not against `public`.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn install_creates_schema_and_runs_migrations_scoped_to_it() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let storage = PluginStorage::new(pool.clone());
        let id = plugin_id("cash.random.storagetest.install");

        let dir = tempfile::tempdir().unwrap();
        write_migration(
            dir.path(),
            "1",
            "CREATE TABLE widgets (id INT PRIMARY KEY);",
        );

        let schema = storage.install(&id, dir.path()).await.unwrap();

        // The table exists inside the plugin's schema...
        let in_schema: (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
             WHERE table_schema = $1 AND table_name = 'widgets')",
        )
        .bind(schema.name())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            in_schema.0,
            "widgets table should exist in {}",
            schema.name()
        );

        // ...and not in public.
        let in_public: (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_name = 'widgets')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!in_public.0, "widgets table must not leak into public");

        storage.uninstall(&id, true).await.unwrap();
    }

    /// Ticket item 2: a migration that fails blocks the install — the error
    /// propagates rather than being swallowed, so a caller cannot mistake
    /// this for a successful install.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn a_failing_migration_returns_an_error_instead_of_succeeding() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let storage = PluginStorage::new(pool.clone());
        let id = plugin_id("cash.random.storagetest.failing");

        let dir = tempfile::tempdir().unwrap();
        write_migration(dir.path(), "1", "THIS IS NOT VALID SQL;");

        let result = storage.install(&id, dir.path()).await;
        assert!(
            result.is_err(),
            "a broken migration must not report success"
        );

        storage.uninstall(&id, true).await.unwrap();
    }

    /// Ticket item 3: a connection from `PluginSchema::acquire` only sees the
    /// plugin's own tables for an unqualified name.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn acquired_connection_is_scoped_to_the_plugin_schema() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let storage = PluginStorage::new(pool.clone());
        let id = plugin_id("cash.random.storagetest.scoped");

        let dir = tempfile::tempdir().unwrap();
        write_migration(
            dir.path(),
            "1",
            "CREATE TABLE widgets (id INT PRIMARY KEY); INSERT INTO widgets VALUES (1);",
        );
        let schema = storage.install(&id, dir.path()).await.unwrap();

        let mut tx = schema.acquire().await.unwrap();
        let row = sqlx::query("SELECT id FROM widgets")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        assert_eq!(row.get::<i32, _>("id"), 1);
        tx.commit().await.unwrap();

        storage.uninstall(&id, true).await.unwrap();
    }

    /// Regression test for the search_path leak the automated review caught:
    /// with the pool pinned to a single physical connection, `install`'s
    /// `SET search_path` must not survive past the migration, or the very
    /// next unrelated query borrowing this connection (a core query
    /// assuming the default `public` path, say) would inherit it.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn install_does_not_leak_search_path_to_the_next_pool_borrower() {
        let Some(database_url) = std::env::var("DATABASE_URL").ok() else {
            return;
        };
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .unwrap();
        let storage = PluginStorage::new(pool.clone());
        let id = plugin_id("cash.random.storagetest.installleak");

        let baseline: (String,) = sqlx::query_as("SHOW search_path")
            .fetch_one(&pool)
            .await
            .unwrap();

        let dir = tempfile::tempdir().unwrap();
        write_migration(
            dir.path(),
            "1",
            "CREATE TABLE widgets (id INT PRIMARY KEY);",
        );
        storage.install(&id, dir.path()).await.unwrap();

        let after: (String,) = sqlx::query_as("SHOW search_path")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            after.0, baseline.0,
            "install must reset search_path before releasing its connection back to the pool"
        );

        storage.uninstall(&id, true).await.unwrap();
    }

    /// Same regression, for `PluginSchema::acquire`: its `SET LOCAL` must
    /// not survive past the transaction it was set in, whether the caller
    /// commits or just drops it.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn acquire_does_not_leak_search_path_to_the_next_pool_borrower() {
        let Some(database_url) = std::env::var("DATABASE_URL").ok() else {
            return;
        };
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .unwrap();
        let storage = PluginStorage::new(pool.clone());
        let id = plugin_id("cash.random.storagetest.acquireleak");

        let dir = tempfile::tempdir().unwrap();
        write_migration(
            dir.path(),
            "1",
            "CREATE TABLE widgets (id INT PRIMARY KEY);",
        );
        // Baseline captured before any plugin operation touches the pool,
        // so this isolates acquire's own leak from install's — install runs
        // its own SET search_path too, and asserting against a baseline
        // taken after install would only prove the two leaks match, not
        // that either was absent.
        let baseline: (String,) = sqlx::query_as("SHOW search_path")
            .fetch_one(&pool)
            .await
            .unwrap();

        let schema = storage.install(&id, dir.path()).await.unwrap();

        let mut tx = schema.acquire().await.unwrap();
        sqlx::query("SELECT 1").execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();

        let after: (String,) = sqlx::query_as("SHOW search_path")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            after.0, baseline.0,
            "acquire's SET LOCAL must not survive past its transaction"
        );

        storage.uninstall(&id, true).await.unwrap();
    }

    /// Ticket item 5: uninstalling without asking to drop the schema leaves
    /// its data behind.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn uninstall_without_drop_schema_keeps_the_data() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let storage = PluginStorage::new(pool.clone());
        let id = plugin_id("cash.random.storagetest.keepdata");

        let dir = tempfile::tempdir().unwrap();
        write_migration(
            dir.path(),
            "1",
            "CREATE TABLE widgets (id INT PRIMARY KEY);",
        );
        let schema = storage.install(&id, dir.path()).await.unwrap();

        storage.uninstall(&id, false).await.unwrap();

        let still_there: (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
             WHERE table_schema = $1 AND table_name = 'widgets')",
        )
        .bind(schema.name())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(still_there.0, "opt-in uninstall must not drop the schema");

        // Clean up for real so the shared test database doesn't accumulate
        // schemas across runs.
        storage.uninstall(&id, true).await.unwrap();
    }

    #[test]
    fn schema_name_prefixes_and_preserves_the_plugin_id() {
        let id = plugin_id("cash.random.billing");
        assert_eq!(schema_name(&id).unwrap(), "plugin_cash.random.billing");
    }

    #[test]
    fn schema_name_refuses_an_id_that_would_be_truncated() {
        let long_id = "a".repeat(60);
        let id = plugin_id(&long_id);
        assert!(matches!(
            schema_name(&id),
            Err(PluginStorageError::SchemaNameTooLong(_))
        ));
    }
}
