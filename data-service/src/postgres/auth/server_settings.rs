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
            SELECT default_confirmations, invoice_expiry_minutes, rate_limit_rpm,
                   -- Cast, and it is load-bearing. The column is `caip2[]` - an
                   -- array of a DOMAIN over text - and sqlx decodes by type OID,
                   -- so asking for `Vec<String>` off a `caip2[]` fails every
                   -- time regardless of what the values are. Reading a settings
                   -- row therefore never worked; it was only ever survivable
                   -- because no row existed, so `fetch_optional` returned
                   -- `None` and the decode never ran.
                   enabled_chain_ids::text[] AS enabled_chain_ids,
                   billing_store_id
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
            // Read as TEXT[] and validate each. The `caip2` domain already
            // enforces the grammar, so a failure here means the column was
            // altered out from under us - treated like any other schema
            // mismatch rather than silently dropping a chain the server is
            // meant to be serving.
            // Skipping a malformed element would silently narrow the set of
            // chains this server believes it serves, so an unparseable one is
            // dropped from the list rather than panicking the connection - and
            // logged, because it means the column was altered underneath us.
            // `try_get`, not `get`. A decode failure here is a schema
            // mismatch, and the previous `get` turned that into a panic
            // inside the connection - which for a value read during boot
            // means the process does not start, and keeps not starting. An
            // instance that will not boot is the one state an operator
            // cannot fix anything else from.
            enabled_chain_ids: r
                .try_get::<Vec<String>, _>("enabled_chain_ids")
                .unwrap_or_else(|e| {
                    tracing::error!(
                        error = %e,
                        "server_settings.enabled_chain_ids could not be read; \
                         continuing with no chains enabled from settings"
                    );
                    Vec::new()
                })
                .into_iter()
                .filter_map(|id| match types::ChainId::parse(id.as_str()) {
                    Ok(chain) => Some(chain),
                    Err(e) => {
                        tracing::error!(
                            value = %id,
                            error = %e,
                            "server_settings.enabled_chain_ids holds a value that is not a CAIP-2 chain id"
                        );
                        None
                    }
                })
                .collect(),
            billing_store_id: r
                .get::<Option<uuid::Uuid>, _>("billing_store_id")
                .map(types::StoreId),
        }))
    }

    async fn upsert_server_settings(&self, settings: &ServerSettings) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO server_settings (id, default_confirmations, invoice_expiry_minutes, rate_limit_rpm, enabled_chain_ids, billing_store_id, updated_at)
            VALUES (1, $1, $2, $3, $4, $5, NOW())
            ON CONFLICT (id) DO UPDATE SET
                default_confirmations = EXCLUDED.default_confirmations,
                invoice_expiry_minutes = EXCLUDED.invoice_expiry_minutes,
                rate_limit_rpm = EXCLUDED.rate_limit_rpm,
                enabled_chain_ids = EXCLUDED.enabled_chain_ids,
                billing_store_id = EXCLUDED.billing_store_id,
                updated_at = NOW()
            "#,
        )
        .bind(settings.default_confirmations)
        .bind(settings.invoice_expiry_minutes)
        .bind(settings.rate_limit_rpm)
        // Bound as TEXT[]; the column's `caip2` domain re-checks each element
        // on the way in, so an invalid identifier is rejected by the database
        // even if it somehow got past the type.
        .bind(
            settings
                .enabled_chain_ids
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>(),
        )
        .bind(settings.billing_store_id.map(|s| s.0))
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(())
    }
}
