//! `InstalledPluginReader`/`InstalledPluginWriter` against `installed_plugins`
//! and `plugin_events`.

use async_trait::async_trait;
use sqlx::Row;
use types::RepositoryResult;

use crate::installed_plugins::{
    InstalledPlugin, InstalledPluginReader, InstalledPluginWriter, NewInstalledPlugin,
    NewPluginEvent, PluginEvent,
};
use crate::sqlx_to_repo_error;

use super::PgDataService;

fn row_to_plugin(row: &sqlx::postgres::PgRow) -> InstalledPlugin {
    InstalledPlugin {
        id: row.get("id"),
        version: row.get("version"),
        manifest_toml: row.get("manifest_toml"),
        artifact_sha256: row.get("artifact_sha256"),
        enabled: row.get("enabled"),
        disabled_reason: row.get("disabled_reason"),
        installed_at: row.get("installed_at"),
        updated_at: row.get("updated_at"),
    }
}

const PLUGIN_COLS: &str = "id, version, manifest_toml, artifact_sha256, enabled, \
                           disabled_reason, installed_at, updated_at";

#[async_trait]
impl InstalledPluginReader for PgDataService {
    async fn list_installed_plugins(&self) -> RepositoryResult<Vec<InstalledPlugin>> {
        let query = format!("SELECT {PLUGIN_COLS} FROM installed_plugins ORDER BY id");
        let rows = sqlx::query(&query)
            .fetch_all(self.pool())
            .await
            .map_err(sqlx_to_repo_error)?;
        Ok(rows.iter().map(row_to_plugin).collect())
    }

    async fn get_installed_plugin(&self, id: &str) -> RepositoryResult<Option<InstalledPlugin>> {
        let query = format!("SELECT {PLUGIN_COLS} FROM installed_plugins WHERE id = $1");
        let row = sqlx::query(&query)
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(sqlx_to_repo_error)?;
        Ok(row.as_ref().map(row_to_plugin))
    }

    async fn plugin_events(&self, id: &str, limit: i64) -> RepositoryResult<Vec<PluginEvent>> {
        let rows = sqlx::query(
            "SELECT plugin_id, event, version, detail, actor_user_id, at \
             FROM plugin_events WHERE plugin_id = $1 ORDER BY at DESC, id DESC LIMIT $2",
        )
        .bind(id)
        .bind(limit)
        .fetch_all(self.pool())
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(rows
            .iter()
            .map(|row| PluginEvent {
                plugin_id: row.get("plugin_id"),
                event: row.get("event"),
                version: row.get("version"),
                detail: row.get("detail"),
                actor_user_id: row.get("actor_user_id"),
                at: row.get("at"),
            })
            .collect())
    }
}

#[async_trait]
impl InstalledPluginWriter for PgDataService {
    async fn upsert_installed_plugin(&self, plugin: &NewInstalledPlugin) -> RepositoryResult<()> {
        // An upgrade re-enables and clears the reason: the admin is
        // installing a different build, and a new version inheriting the old
        // one's disable would make the single action most likely to fix a
        // broken plugin unable to.
        sqlx::query(
            "INSERT INTO installed_plugins \
                 (id, version, manifest_toml, artifact_sha256, enabled, disabled_reason) \
             VALUES ($1, $2, $3, $4, TRUE, NULL) \
             ON CONFLICT (id) DO UPDATE SET \
                 version = EXCLUDED.version, \
                 manifest_toml = EXCLUDED.manifest_toml, \
                 artifact_sha256 = EXCLUDED.artifact_sha256, \
                 enabled = TRUE, \
                 disabled_reason = NULL, \
                 updated_at = NOW()",
        )
        .bind(&plugin.id)
        .bind(&plugin.version)
        .bind(&plugin.manifest_toml)
        .bind(&plugin.artifact_sha256)
        .execute(self.pool())
        .await
        .map_err(sqlx_to_repo_error)?;
        Ok(())
    }

    async fn set_plugin_enabled(
        &self,
        id: &str,
        enabled: bool,
        reason: Option<&str>,
    ) -> RepositoryResult<()> {
        // Enabling always clears the reason, whatever the caller passed: a
        // plugin that is on must not still carry an explanation for being
        // off, which an admin would reasonably read as still being off.
        let stored_reason = if enabled { None } else { reason };
        sqlx::query(
            "UPDATE installed_plugins \
             SET enabled = $2, disabled_reason = $3, updated_at = NOW() \
             WHERE id = $1",
        )
        .bind(id)
        .bind(enabled)
        .bind(stored_reason)
        .execute(self.pool())
        .await
        .map_err(sqlx_to_repo_error)?;
        Ok(())
    }

    async fn remove_installed_plugin(&self, id: &str) -> RepositoryResult<bool> {
        let result = sqlx::query("DELETE FROM installed_plugins WHERE id = $1")
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(sqlx_to_repo_error)?;
        Ok(result.rows_affected() > 0)
    }

    async fn record_plugin_event(&self, event: &NewPluginEvent) -> RepositoryResult<()> {
        sqlx::query(
            "INSERT INTO plugin_events (plugin_id, event, version, detail, actor_user_id) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&event.plugin_id)
        .bind(event.kind.as_str())
        .bind(&event.version)
        .bind(&event.detail)
        .bind(event.actor_user_id)
        .execute(self.pool())
        .await
        .map_err(sqlx_to_repo_error)?;
        Ok(())
    }
}
