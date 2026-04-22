//! Store settings repository implementation.

use async_trait::async_trait;
use sqlx::Row;
use uuid::Uuid;

use super::PgDataService;
use crate::{
    RepositoryResult, StoreSettings, StoreSettingsReader, StoreSettingsWriter, sqlx_to_repo_error,
};

fn row_to_settings(row: &sqlx::postgres::PgRow) -> StoreSettings {
    StoreSettings {
        store_id: row.get("store_id"),
        default_chain_id: row.get("default_chain_id"),
        default_display_currency: row.get("default_display_currency"),
        logo_url: row.get("logo_url"),
        accent_color: row.get("accent_color"),
        notification_prefs: row.get("notification_prefs"),
        updated_at: row.get("updated_at"),
    }
}

#[async_trait]
impl StoreSettingsReader for PgDataService {
    async fn get_store_settings(&self, store_id: Uuid) -> RepositoryResult<Option<StoreSettings>> {
        let row = sqlx::query("SELECT * FROM store_settings WHERE store_id = $1")
            .bind(store_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        Ok(row.as_ref().map(row_to_settings))
    }
}

#[async_trait]
impl StoreSettingsWriter for PgDataService {
    async fn upsert_store_settings(
        &self,
        store_id: Uuid,
        default_chain_id: Option<i64>,
        default_display_currency: Option<&str>,
        logo_url: Option<&str>,
        accent_color: Option<&str>,
        notification_prefs: &serde_json::Value,
    ) -> RepositoryResult<StoreSettings> {
        let row = sqlx::query(
            r#"
            INSERT INTO store_settings (store_id, default_chain_id, default_display_currency, logo_url, accent_color, notification_prefs)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (store_id) DO UPDATE SET
                default_chain_id = EXCLUDED.default_chain_id,
                default_display_currency = EXCLUDED.default_display_currency,
                logo_url = EXCLUDED.logo_url,
                accent_color = EXCLUDED.accent_color,
                notification_prefs = EXCLUDED.notification_prefs,
                updated_at = NOW()
            RETURNING *
            "#,
        )
        .bind(store_id)
        .bind(default_chain_id)
        .bind(default_display_currency)
        .bind(logo_url)
        .bind(accent_color)
        .bind(notification_prefs)
        .fetch_one(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row_to_settings(&row))
    }
}
