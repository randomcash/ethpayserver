//! DeviceRepository implementation.

use async_trait::async_trait;
use sqlx::Row;

use auth::{Device, DeviceId, DeviceRepository, UserId, error::{AuthError, Result}};

use super::{PgDataService, db_to_device_type, device_type_to_db, sqlx_to_auth_error};

#[async_trait]
impl DeviceRepository for PgDataService {
    async fn create_device(&self, device: &Device) -> Result<()> {
        let kdf_params = serde_json::to_value(&device.kdf_params)
            .map_err(|e| AuthError::Repository(e.to_string()))?;
        let encrypted_key = serde_json::to_value(&device.encrypted_symmetric_key)
            .map_err(|e| AuthError::Repository(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO devices (
                id, user_id, name, device_type, encrypted_symmetric_key, kdf_params,
                created_at, last_used_at, is_active
            ) VALUES ($1, $2, $3, $4::device_type, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(device.id.0)
        .bind(device.user_id.0)
        .bind(&device.name)
        .bind(device_type_to_db(device.device_type))
        .bind(&encrypted_key)
        .bind(&kdf_params)
        .bind(device.created_at)
        .bind(device.last_used_at)
        .bind(device.is_active)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(())
    }

    async fn get_device(&self, id: DeviceId) -> Result<Option<Device>> {
        let row = sqlx::query(
            r#"
            SELECT id, user_id, name, device_type::text, encrypted_symmetric_key, kdf_params,
                   created_at, last_used_at, is_active
            FROM devices WHERE id = $1
            "#,
        )
        .bind(id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        row.map(|r| row_to_device(&r)).transpose()
    }

    async fn get_devices_for_user(&self, user_id: UserId) -> Result<Vec<Device>> {
        let rows = sqlx::query(
            r#"
            SELECT id, user_id, name, device_type::text, encrypted_symmetric_key, kdf_params,
                   created_at, last_used_at, is_active
            FROM devices WHERE user_id = $1
            "#,
        )
        .bind(user_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        rows.iter().map(row_to_device).collect()
    }

    async fn update_device(&self, device: &Device) -> Result<()> {
        let kdf_params = serde_json::to_value(&device.kdf_params)
            .map_err(|e| AuthError::Repository(e.to_string()))?;
        let encrypted_key = serde_json::to_value(&device.encrypted_symmetric_key)
            .map_err(|e| AuthError::Repository(e.to_string()))?;

        let result = sqlx::query(
            r#"
            UPDATE devices SET
                name = $2, device_type = $3::device_type, encrypted_symmetric_key = $4,
                kdf_params = $5, last_used_at = $6, is_active = $7
            WHERE id = $1
            "#,
        )
        .bind(device.id.0)
        .bind(&device.name)
        .bind(device_type_to_db(device.device_type))
        .bind(&encrypted_key)
        .bind(&kdf_params)
        .bind(device.last_used_at)
        .bind(device.is_active)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        if result.rows_affected() == 0 {
            return Err(AuthError::DeviceNotFound(device.id.to_string()));
        }
        Ok(())
    }

    async fn deactivate_device(&self, id: DeviceId) -> Result<()> {
        let result = sqlx::query("UPDATE devices SET is_active = FALSE WHERE id = $1")
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_auth_error)?;

        if result.rows_affected() == 0 {
            return Err(AuthError::DeviceNotFound(id.to_string()));
        }
        Ok(())
    }

    async fn delete_device(&self, id: DeviceId) -> Result<()> {
        sqlx::query("DELETE FROM devices WHERE id = $1")
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_auth_error)?;
        Ok(())
    }

    async fn delete_all_devices_for_user(&self, user_id: UserId) -> Result<()> {
        sqlx::query("DELETE FROM devices WHERE user_id = $1")
            .bind(user_id.0)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_auth_error)?;
        Ok(())
    }

    async fn count_active_devices(&self, user_id: UserId) -> Result<u32> {
        let row = sqlx::query(
            "SELECT COUNT(*) as count FROM devices WHERE user_id = $1 AND is_active = TRUE",
        )
        .bind(user_id.0)
        .fetch_one(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        let count: i64 = row.get("count");
        Ok(count as u32)
    }
}

fn row_to_device(row: &sqlx::postgres::PgRow) -> Result<Device> {
    let kdf_params_json: serde_json::Value = row.get("kdf_params");
    let encrypted_key_json: serde_json::Value = row.get("encrypted_symmetric_key");
    let device_type_str: String = row.get("device_type");

    Ok(Device {
        id: DeviceId(row.get("id")),
        user_id: UserId(row.get("user_id")),
        name: row.get("name"),
        device_type: db_to_device_type(&device_type_str),
        encrypted_symmetric_key: serde_json::from_value(encrypted_key_json)
            .map_err(|e| AuthError::Repository(e.to_string()))?,
        kdf_params: serde_json::from_value(kdf_params_json)
            .map_err(|e| AuthError::Repository(e.to_string()))?,
        created_at: row.get("created_at"),
        last_used_at: row.get("last_used_at"),
        is_active: row.get("is_active"),
    })
}
