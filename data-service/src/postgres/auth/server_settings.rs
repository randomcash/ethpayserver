//! ServerSettingsRepository implementation.

use async_trait::async_trait;
use sqlx::Row;

use auth::{ServerSettings, ServerSettingsRepository, error::Result};

use super::{PgDataService, sqlx_to_auth_error};

#[async_trait]
impl ServerSettingsRepository for PgDataService {
    async fn get_server_settings(&self) -> Result<Option<ServerSettings>> {
        let row = sqlx::query(
            r#"
            SELECT default_confirmations, invoice_expiry_minutes, rate_limit_rpm, enabled_chain_ids
            FROM server_settings WHERE id = 1
            "#,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(row.map(|r| ServerSettings {
            default_confirmations: r.get("default_confirmations"),
            invoice_expiry_minutes: r.get("invoice_expiry_minutes"),
            rate_limit_rpm: r.get("rate_limit_rpm"),
            enabled_chain_ids: r.get("enabled_chain_ids"),
        }))
    }

    async fn upsert_server_settings(&self, settings: &ServerSettings) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO server_settings (id, default_confirmations, invoice_expiry_minutes, rate_limit_rpm, enabled_chain_ids, updated_at)
            VALUES (1, $1, $2, $3, $4, NOW())
            ON CONFLICT (id) DO UPDATE SET
                default_confirmations = EXCLUDED.default_confirmations,
                invoice_expiry_minutes = EXCLUDED.invoice_expiry_minutes,
                rate_limit_rpm = EXCLUDED.rate_limit_rpm,
                enabled_chain_ids = EXCLUDED.enabled_chain_ids,
                updated_at = NOW()
            "#,
        )
        .bind(settings.default_confirmations)
        .bind(settings.invoice_expiry_minutes)
        .bind(settings.rate_limit_rpm)
        .bind(&settings.enabled_chain_ids)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(())
    }
}
