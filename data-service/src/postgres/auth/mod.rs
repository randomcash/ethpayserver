//! Auth repository implementations for PostgreSQL.

mod api_key;
mod api_key_impl;
mod challenge;
mod device;
mod passkey;
mod session;
mod store;
mod user;
mod wallet;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod integration_tests;

use auth::{DeviceType, error::AuthError};

// Re-export for use in submodules
pub(super) use super::PgDataService;

pub(crate) fn device_type_to_db(dt: DeviceType) -> &'static str {
    match dt {
        DeviceType::Browser => "browser",
        DeviceType::Desktop => "desktop",
        DeviceType::Mobile => "mobile",
        DeviceType::ApiClient => "api_client",
        DeviceType::Unknown => "unknown",
    }
}

pub(crate) fn db_to_device_type(s: &str) -> DeviceType {
    match s {
        "browser" => DeviceType::Browser,
        "desktop" => DeviceType::Desktop,
        "mobile" => DeviceType::Mobile,
        "api_client" => DeviceType::ApiClient,
        _ => DeviceType::Unknown,
    }
}

pub(crate) fn sqlx_to_auth_error(e: sqlx::Error) -> AuthError {
    match e {
        sqlx::Error::RowNotFound => AuthError::UserNotFound("not found".into()),
        sqlx::Error::Database(ref db_err) => {
            if db_err.is_unique_violation() {
                AuthError::UserExists("already exists".into())
            } else {
                AuthError::Repository(e.to_string())
            }
        }
        _ => AuthError::Repository(e.to_string()),
    }
}
