//! What is installed, and what the host did about it.
//!
//! The plugin host is in-memory by construction: `PluginHost` compiles and
//! holds wasmtime instances, and every one of them dies with the process.
//! This is the part that outlives a boot - which plugins an admin installed,
//! which artifact each one is, and whether the host turned one off.
//!
//! Lives here rather than in `payserver-commons` for the same reason
//! `PaymentAnalyticsReader` does: it is this server's own operational state,
//! not part of the payment-server contract every backend has to satisfy.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use types::RepositoryResult;
use uuid::Uuid;

/// An installed plugin as the database records it.
///
/// The manifest travels as its original TOML text rather than a parsed
/// struct: `payserver_plugin_api::Manifest` is `Deserialize` only, and
/// re-parsing the exact bytes that were accepted at install means boot runs
/// the same gate the install ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPlugin {
    pub id: String,
    pub version: String,
    pub manifest_toml: String,
    pub artifact_sha256: String,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    /// The password for this plugin's own database login role.
    ///
    /// `None` for a plugin installed before per-plugin roles existed, or one
    /// whose provisioning did not complete. Such a plugin gets no database
    /// access at all rather than falling back to the host's connection -
    /// which is the direction that cannot leak.
    pub db_role_password: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// What to write when a plugin is installed or upgraded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewInstalledPlugin {
    pub id: String,
    pub version: String,
    pub manifest_toml: String,
    pub artifact_sha256: String,
    /// The password for the login role the plugin's own statements run as.
    pub db_role_password: Option<String>,
}

/// The lifecycle events worth keeping after the fact.
///
/// `LoadFailed` is the one the host writes to itself: a plugin that did not
/// come up is disabled and the reason recorded, so the next boot does not
/// retry it and an admin can read what happened without the logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginEventKind {
    Installed,
    Upgraded,
    Enabled,
    Disabled,
    Uninstalled,
    LoadFailed,
}

impl PluginEventKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Upgraded => "upgraded",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::Uninstalled => "uninstalled",
            Self::LoadFailed => "load_failed",
        }
    }
}

/// One audit row.
///
/// `actor_user_id` is `None` when the host acted on its own - a
/// crash-disable has no admin behind it, and attributing it to whoever was
/// logged in at the time would be a lie the audit trail cannot correct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPluginEvent {
    pub plugin_id: String,
    pub kind: PluginEventKind,
    pub version: Option<String>,
    pub detail: Option<String>,
    pub actor_user_id: Option<Uuid>,
}

/// One audit row as stored, for the admin view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginEvent {
    pub plugin_id: String,
    pub event: String,
    pub version: Option<String>,
    pub detail: Option<String>,
    pub actor_user_id: Option<Uuid>,
    pub at: DateTime<Utc>,
}

#[async_trait]
pub trait InstalledPluginReader: Send + Sync {
    /// Every installed plugin, disabled ones included.
    ///
    /// Boot needs the disabled rows too: it must know a plugin exists and is
    /// deliberately off, so it can say so, rather than treating "absent" and
    /// "switched off after crashing" as the same thing.
    async fn list_installed_plugins(&self) -> RepositoryResult<Vec<InstalledPlugin>>;

    async fn get_installed_plugin(&self, id: &str) -> RepositoryResult<Option<InstalledPlugin>>;

    /// The most recent events for one plugin, newest first.
    async fn plugin_events(&self, id: &str, limit: i64) -> RepositoryResult<Vec<PluginEvent>>;
}

#[async_trait]
pub trait InstalledPluginWriter: Send + Sync {
    /// Install, or upgrade in place.
    ///
    /// An upgrade clears `disabled_reason` and re-enables: the admin is
    /// installing a different build, and holding a new version responsible
    /// for the previous one's crash would make a plugin unrecoverable by the
    /// one action that is most likely to fix it.
    async fn upsert_installed_plugin(&self, plugin: &NewInstalledPlugin) -> RepositoryResult<()>;

    /// Turn a plugin on or off for future boots.
    ///
    /// `reason` is recorded only when disabling; enabling clears it.
    ///
    /// Returns whether a row was actually updated, the same way
    /// [`Self::remove_installed_plugin`] does. The boot loader always has a
    /// row in hand, but the admin endpoint this exists for takes a plugin id
    /// from a request, and reporting success for an id that does not exist
    /// is how an admin concludes they have disabled something they have not.
    async fn set_plugin_enabled(
        &self,
        id: &str,
        enabled: bool,
        reason: Option<&str>,
    ) -> RepositoryResult<bool>;

    /// Returns whether a row was actually removed.
    async fn remove_installed_plugin(&self, id: &str) -> RepositoryResult<bool>;

    async fn record_plugin_event(&self, event: &NewPluginEvent) -> RepositoryResult<()>;
}
