//! Loading, at startup, the plugins an admin installed.
//!
//! Until this module existed, `PluginHost` was constructed in tests and
//! nowhere else: a manifest could be gated, a module compiled, an action
//! dispatched and a filter consulted, and none of it was reachable from a
//! running server, because nothing ever built a host or told it what to
//! load. This is the seam that closes - the database says what is installed,
//! the artifact directory holds the wasm, and this turns the two into a
//! registered plugin.
//!
//! ## A failure here disables the plugin, it does not stop the boot
//!
//! A plugin that will not load is the admin's problem to fix, and the server
//! refusing to start is the worst possible way to tell them: the UI they
//! would fix it from is the thing that did not come up. BTCPay's users hit
//! exactly this and their documented fallback is deleting the plugin out of
//! a Docker volume by hand. So a load failure is recorded, the plugin is
//! disabled, and the server comes up without it.
//!
//! ## And the disable is written down
//!
//! Disabling a crashed plugin only in memory is undone by the restart that
//! disabling it was supposed to make safe - the plugin comes back, fails
//! again, and the server is in a crash loop that looks like a mystery. The
//! row is updated, so the next boot skips it and an admin can see why
//! without reading logs.

use data_service::{
    InstalledPlugin, InstalledPluginReader, InstalledPluginWriter, NewPluginEvent, PluginEventKind,
};
use payserver_plugin_api::{Manifest, PluginId};

use payserver_plugin_host::PluginHost;
use payserver_plugin_host::{ArtifactError, PluginArtifacts};

/// A plugin that did not load, and what was done about it.
#[derive(Debug, PartialEq, Eq)]
pub struct PluginLoadFailure {
    pub id: PluginId,
    /// The admin-facing reason, already rendered.
    pub reason: String,
    /// Whether the plugin was switched off for future boots.
    ///
    /// Only for failures that are properties of the plugin itself. See
    /// [`FailureKind`].
    pub disabled: bool,
}

/// Whether a load failure says something about the plugin, or about the
/// machine it happens to be running on.
///
/// The distinction decides whether the failure is written down. Disabling a
/// plugin is the right answer to a broken plugin and the wrong answer to a
/// broken mount: a container that restarts before its plugin volume is
/// attached would otherwise switch off everything installed, and attaching
/// the volume would not bring any of it back - recovery would mean editing
/// the database by hand, which is the exact outcome this module exists to
/// avoid.
///
/// So the rule is: persist a disable only when retrying could not possibly
/// help. A missing file can become present again; a digest that does not
/// match never will.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    /// The artifact or its install record is wrong. Retrying is pointless
    /// and running it is not an option, so it is disabled for future boots.
    Plugin,
    /// The plugin could not be reached - no directory, no permission, an I/O
    /// error. Nothing about the plugin is known to be wrong, so the next
    /// boot tries again.
    Environment,
}

/// Consecutive failed calls before the host disables a plugin by itself.
///
/// Low on purpose. A plugin that has trapped three times in a row is not
/// having a bad moment, and the cost of being wrong is asymmetric: a
/// wrongly-disabled plugin is one admin click from coming back, while a
/// plugin left enabled through a crash loop takes a request path down with
/// it every time it is consulted.
pub const DEFAULT_MAX_FAILURES: u32 = 3;

/// How long any single plugin call may run before the epoch deadline fires.
///
/// One knob for both call shapes today, set by the stricter of the two: an
/// invoice-creation filter runs inline on a request a merchant is waiting
/// on, so seconds here are seconds of latency on invoice creation. Actions
/// are fire-and-forget and could tolerate more, and will want their own
/// deadline once anything actually needs it - splitting it before then would
/// be two numbers to tune with no evidence about either.
pub const DEFAULT_CALL_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);

/// What a boot did about each installed plugin.
///
/// Returned rather than only logged so the caller can report it and a test
/// can assert on it; the counts are what an admin page summarises.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PluginBootReport {
    /// Compiled, instantiated and registered with the host.
    ///
    /// These ids are what the boot hands to `dispatch::invoice_creation_filters`
    /// and `dispatch::payment_observers` to build the capability
    /// implementations the rest of the server calls, so a plugin listed here
    /// is reachable: a filter among them is consulted before every invoice is
    /// created, and any of them may be told that an own-store invoice settled.
    ///
    /// Reachable is still not the same as *called*. A filter is only offered
    /// the hook if its manifest declares `kind = "filter"`, and own-store
    /// payment reporting additionally needs `ETHPAY_BILLING_STORE_ID` set.
    pub loaded: Vec<PluginId>,
    /// Installed but switched off - by an admin, or by a previous boot that
    /// could not load it. Carries the recorded reason where there is one.
    pub skipped: Vec<(PluginId, Option<String>)>,
    /// Failed this boot. Whether each one was also switched off for future
    /// boots depends on what went wrong - see [`PluginLoadFailure`].
    pub failed: Vec<PluginLoadFailure>,
    /// Every plugin was skipped because this boot is in safe mode.
    pub safe_mode: bool,
}

impl PluginBootReport {
    #[must_use]
    pub fn total(&self) -> usize {
        self.loaded.len() + self.skipped.len() + self.failed.len()
    }
}

/// Load every enabled installed plugin into `host`.
///
/// `host` is `None` in safe mode, because in safe mode there is no host:
/// that boot builds no wasmtime engine at all. Representing safe mode as the
/// absence of the thing that runs plugins, rather than as a flag next to it,
/// means there is no state where a host exists and a caller has forgotten to
/// check whether it was supposed to use it.
///
/// Safe mode short-circuits before any row is touched: it is a property of
/// this boot, not a disable. Nothing is written, so clearing the flag and
/// restarting brings every plugin back exactly as it was - which is the
/// whole point of having a way in that does not require disk access.
pub async fn load_installed_plugins<D>(
    data: &D,
    host: Option<&PluginHost>,
    artifacts: &PluginArtifacts,
    pools: Option<&std::sync::Arc<super::PluginPools>>,
    issuer: &super::DeferredIssuer,
) -> Result<PluginBootReport, types::RepositoryError>
where
    D: InstalledPluginReader + InstalledPluginWriter + ?Sized,
{
    let installed = data.list_installed_plugins().await?;
    let mut report = PluginBootReport {
        safe_mode: host.is_none(),
        ..Default::default()
    };

    for row in installed {
        // An id that no longer parses is a database that has been edited by
        // hand, or a `PluginId` whose rules tightened since the install. It
        // cannot be turned into a `PluginId` to report against, so it is
        // logged and skipped rather than silently dropped.
        let Ok(id) = PluginId::new(row.id.clone()) else {
            tracing::error!(
                plugin_id = %row.id,
                "installed_plugins holds an id that is not a valid plugin id; skipping it"
            );
            continue;
        };

        let Some(host) = host else {
            report.skipped.push((id, Some("safe mode".to_string())));
            continue;
        };

        if !row.enabled {
            report.skipped.push((id, row.disabled_reason.clone()));
            continue;
        }

        match register_or_disable(data, host, artifacts, &id, &row, pools, issuer).await {
            Ok(()) => report.loaded.push(id),
            Err(failure) => report.failed.push(failure),
        }
    }

    Ok(report)
}

/// Register one plugin, or record the failure - and, when the plugin itself
/// is what is wrong, switch it off for future boots.
///
/// The disable happens here rather than at the call site so that failing to
/// load and being switched off for next time are one step: a boot that
/// reported a broken plugin without recording it would retry the same broken
/// plugin on every restart, which is the crash loop this module exists to
/// avoid. An environment failure is the opposite case and is deliberately
/// left retryable - see [`FailureKind`].
async fn register_or_disable<D>(
    data: &D,
    host: &PluginHost,
    artifacts: &PluginArtifacts,
    id: &PluginId,
    row: &InstalledPlugin,
    pools: Option<&std::sync::Arc<super::PluginPools>>,
    issuer: &super::DeferredIssuer,
) -> Result<(), PluginLoadFailure>
where
    D: InstalledPluginWriter + ?Sized,
{
    let calls = host_calls_for(id, row, pools, issuer).await;

    let Err((kind, reason)) = load_one(
        host,
        artifacts,
        id,
        &row.version,
        &row.manifest_toml,
        &row.artifact_sha256,
        calls,
    ) else {
        tracing::info!(plugin_id = %id, version = %row.version, "plugin loaded");
        return Ok(());
    };

    let disabled = kind == FailureKind::Plugin;
    if disabled {
        tracing::error!(
            plugin_id = %id,
            version = %row.version,
            reason = %reason,
            "plugin failed to load; disabling it so the next boot does not retry"
        );
    } else {
        tracing::error!(
            plugin_id = %id,
            version = %row.version,
            reason = %reason,
            "plugin could not be read; leaving it enabled so it loads once whatever \
             is holding the artifact is fixed"
        );
    }

    record_failure(data, &row.id, &row.version, &reason, disabled).await;
    Err(PluginLoadFailure {
        id: id.clone(),
        reason,
        disabled,
    })
}

/// Parse, verify and register one plugin.
///
/// The error carries the admin-facing reason already rendered, because that
/// is what gets stored, and the classification that decides whether it is
/// stored at all.
fn load_one(
    host: &PluginHost,
    artifacts: &PluginArtifacts,
    id: &PluginId,
    version: &str,
    manifest_toml: &str,
    expected_sha256: &str,
    calls: Option<std::sync::Arc<dyn payserver_plugin_host::PluginHostCalls>>,
) -> Result<(), (FailureKind, String)> {
    let manifest: Manifest = manifest_toml.parse().map_err(|e| {
        (
            FailureKind::Plugin,
            format!("stored manifest no longer parses: {e}"),
        )
    })?;

    // The manifest is the authority on the plugin's own id, and the row is
    // the authority on which artifact was installed. If they disagree, the
    // install record does not describe the thing on disk and neither answer
    // is safe to act on.
    if manifest.id != *id {
        return Err((
            FailureKind::Plugin,
            format!(
                "stored manifest declares id {}, but it is installed as {id}",
                manifest.id
            ),
        ));
    }
    if manifest.version.to_string() != version {
        return Err((
            FailureKind::Plugin,
            format!(
                "stored manifest declares version {}, but it is installed as {version}",
                manifest.version
            ),
        ));
    }

    let wasm = artifacts
        .read_verified(id, version, expected_sha256)
        .map_err(|e| (classify(&e), e.to_string()))?;

    host.register_with_calls(manifest, &wasm, calls)
        .map_err(|e| (FailureKind::Plugin, format!("host refused it: {e}")))
}

/// The host calls this plugin gets: its database, if it has a credential and
/// this boot has pools, and the invoice issuer, whenever one is published.
///
/// The database half is required and not assumed:
///
/// - A plugin installed before per-plugin roles existed has no password, and
///   gets no database rather than the host's connection. There is no safe
///   fallback: the host connects as a superuser, so absent means absent.
/// - Registering a pool opens no connection (see [`PluginPools::register`]),
///   so the only way it fails is a `DATABASE_URL` this process could not
///   parse - which the rest of the boot has already survived. It is logged
///   and treated as no database, so one plugin's bad credential does not stop
///   the others loading.
///
/// A plugin with no database gets no host calls at all, invoicing included.
/// That is deliberate rather than incidental: a plugin that cannot record
/// what it issued must not be able to issue. Billing that charges a merchant
/// and loses the fact would be worse than billing that does not run.
async fn host_calls_for(
    id: &PluginId,
    row: &InstalledPlugin,
    pools: Option<&std::sync::Arc<super::PluginPools>>,
    issuer: &super::DeferredIssuer,
) -> Option<std::sync::Arc<dyn payserver_plugin_host::PluginHostCalls>> {
    let (pools, password) = (pools?, row.db_role_password.as_deref()?);

    match pools.register(id, password).await {
        Ok(()) => Some(std::sync::Arc::new(
            super::PluginCalls::new(id.clone(), std::sync::Arc::clone(pools))
                .with_issuer(issuer.clone()),
        )
            as std::sync::Arc<dyn payserver_plugin_host::PluginHostCalls>),
        Err(e) => {
            tracing::error!(
                plugin_id = %id,
                error = %e,
                "could not give this plugin database access; it will load without it"
            );
            None
        }
    }
}

/// Which failures are the plugin's and which are the machine's.
///
/// A digest that does not match will not start matching, and a version that
/// cannot be a filename will not become one - those are the plugin's. A file
/// that is missing or unreadable may well be there on the next boot, once a
/// volume is mounted or a permission fixed.
fn classify(err: &ArtifactError) -> FailureKind {
    match err {
        ArtifactError::DigestMismatch { .. } | ArtifactError::UnsafeVersion(_) => {
            FailureKind::Plugin
        }
        ArtifactError::NotFound(_) | ArtifactError::Unreadable { .. } => FailureKind::Environment,
        // A write error cannot occur on the read path this classifies, but
        // it is the machine's either way.
        ArtifactError::Unwritable { .. } => FailureKind::Environment,
    }
}

/// Switch a plugin off for future boots, reporting rather than propagating
/// a failure to do so - the boot is already past the point where this could
/// change anything.
async fn persist_disable<D>(data: &D, id: &str, reason: &str)
where
    D: InstalledPluginWriter + ?Sized,
{
    match data.set_plugin_enabled(id, false, Some(reason)).await {
        Ok(true) => {}
        Ok(false) => tracing::error!(
            plugin_id = %id,
            "could not disable a plugin that failed to load: it is no longer in \
             installed_plugins. It will be retried if it reappears."
        ),
        Err(e) => tracing::error!(
            plugin_id = %id,
            error = %e,
            "could not disable a plugin that failed to load; it will be retried \
             on the next boot"
        ),
    }
}

/// Record the failure, and switch the plugin off for future boots when
/// `disable` says the plugin itself is what is wrong.
///
/// The event is written either way. A repeated environment failure is
/// exactly the thing an admin needs to see the history of - one row per boot
/// is bounded by how often the process restarts, and a silent recurring
/// failure would be worse than a noisy one.
///
/// Deliberately infallible from the caller's point of view: the server is
/// starting, the plugin is already not going to run, and a database write
/// failing here must not be the thing that stops the boot. It is logged
/// loudly instead, because the consequence - a retry on the next boot - is
/// worth knowing about.
async fn record_failure<D>(data: &D, id: &str, version: &str, reason: &str, disable: bool)
where
    D: InstalledPluginWriter + ?Sized,
{
    if disable {
        persist_disable(data, id, reason).await;
    }
    let event = NewPluginEvent {
        plugin_id: id.to_string(),
        kind: PluginEventKind::LoadFailed,
        version: Some(version.to_string()),
        detail: Some(reason.to_string()),
        // No actor: the host did this to itself.
        actor_user_id: None,
    };
    if let Err(e) = data.record_plugin_event(&event).await {
        tracing::error!(plugin_id = %id, error = %e, "could not record a plugin load failure");
    }
}

/// Log what the boot did, at a level that matches how bad it is.
///
/// `cognitive_complexity` is allowed because every branch here is a
/// `tracing` macro, each of which expands to a conditional the lint counts
/// and a reader does not. The function is a flat sequence of log lines.
#[allow(clippy::cognitive_complexity)]
pub fn report_boot(report: &PluginBootReport) {
    if report.safe_mode {
        tracing::warn!(
            installed = report.total(),
            "SAFE MODE: no plugin was loaded. Clear ETHPAY_DISABLE_PLUGINS and restart to \
             bring them back; nothing has been uninstalled."
        );
        return;
    }
    if report.total() == 0 {
        tracing::info!("no plugins installed");
        return;
    }
    tracing::info!(
        loaded = report.loaded.len(),
        skipped = report.skipped.len(),
        failed = report.failed.len(),
        "plugin host ready"
    );
    for failure in &report.failed {
        if failure.disabled {
            tracing::error!(
                plugin_id = %failure.id,
                reason = %failure.reason,
                "plugin disabled after failing to load"
            );
        } else {
            tracing::error!(
                plugin_id = %failure.id,
                reason = %failure.reason,
                "plugin not loaded this boot; still enabled, and will be retried"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::Mutex;

    use async_trait::async_trait;
    use chrono::Utc;
    use types::{RepositoryError, RepositoryResult};

    use payserver_plugin_host::{PluginHost, host_version};

    use super::*;

    /// The smallest module this host will load: the ABI exports and a `call`
    /// that hands its argument straight back.
    ///
    /// Defined here rather than borrowed from `payserver-plugin-host`, whose
    /// equivalent fixtures are `#[cfg(test)]` and so do not cross the crate
    /// boundary. Duplicating fifteen lines of WAT that only has to *load* is
    /// a much smaller cost than the alternative these tests exist alongside -
    /// duplicating the host itself - and this copy asserts nothing about the
    /// runtime's behaviour, only that boot accepts a well-formed artifact.
    fn echo_module() -> Vec<u8> {
        wat::parse_str(
            r#"
            (module
                (memory (export "memory") 1)
                (global $next (mut i32) (i32.const 1024))

                (func (export "alloc") (param $len i32) (result i32)
                    (local $ptr i32)
                    (local.set $ptr (global.get $next))
                    (global.set $next (i32.add (global.get $next) (local.get $len)))
                    (local.get $ptr))

                (func (export "call") (param $ptr i32) (param $len i32) (result i64)
                    (i64.or
                        (i64.shl (i64.extend_i32_u (local.get $ptr)) (i64.const 32))
                        (i64.extend_i32_u (local.get $len))))
            )
            "#,
        )
        .unwrap()
    }

    const PLUGIN_ID: &str = "cash.random.billing";

    /// A manifest that this host will accept, built against the running
    /// `host_version()` rather than a literal, so a crate version bump does
    /// not quietly turn these tests into assertions about version rejection.
    fn manifest_toml(id: &str, version: &str) -> String {
        let host = host_version();
        format!(
            r#"
            id = "{id}"
            version = "{version}"
            dependencies = ["ethpayserver:^{}.{}"]
            kind = "action"
            "#,
            host.major, host.minor
        )
    }

    /// The installed-plugin tables, in memory.
    #[derive(Default)]
    struct FakeStore {
        rows: Mutex<Vec<InstalledPlugin>>,
        events: Mutex<Vec<NewPluginEvent>>,
    }

    impl FakeStore {
        fn with_row(row: InstalledPlugin) -> Self {
            Self {
                rows: Mutex::new(vec![row]),
                events: Mutex::new(Vec::new()),
            }
        }

        fn row(&self, id: &str) -> InstalledPlugin {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .find(|r| r.id == id)
                .cloned()
                .expect("row should still exist")
        }

        fn event_kinds(&self) -> Vec<&'static str> {
            self.events
                .lock()
                .unwrap()
                .iter()
                .map(|e| e.kind.as_str())
                .collect()
        }
    }

    #[async_trait]
    impl InstalledPluginReader for FakeStore {
        async fn list_installed_plugins(&self) -> RepositoryResult<Vec<InstalledPlugin>> {
            Ok(self.rows.lock().unwrap().clone())
        }

        async fn get_installed_plugin(
            &self,
            id: &str,
        ) -> RepositoryResult<Option<InstalledPlugin>> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|r| r.id == id)
                .cloned())
        }

        async fn plugin_events(
            &self,
            _id: &str,
            _limit: i64,
        ) -> RepositoryResult<Vec<data_service::PluginEvent>> {
            Ok(Vec::new())
        }
    }

    #[async_trait]
    impl InstalledPluginWriter for FakeStore {
        async fn upsert_installed_plugin(
            &self,
            _plugin: &data_service::NewInstalledPlugin,
        ) -> RepositoryResult<()> {
            Err(RepositoryError::InvalidData(
                "not needed by these tests".to_string(),
            ))
        }

        async fn set_plugin_enabled(
            &self,
            id: &str,
            enabled: bool,
            reason: Option<&str>,
        ) -> RepositoryResult<bool> {
            let mut rows = self.rows.lock().unwrap();
            let Some(row) = rows.iter_mut().find(|r| r.id == id) else {
                return Ok(false);
            };
            row.enabled = enabled;
            row.disabled_reason = if enabled {
                None
            } else {
                reason.map(str::to_string)
            };
            Ok(true)
        }

        async fn remove_installed_plugin(&self, _id: &str) -> RepositoryResult<bool> {
            Err(RepositoryError::InvalidData(
                "not needed by these tests".to_string(),
            ))
        }

        async fn record_plugin_event(&self, event: &NewPluginEvent) -> RepositoryResult<()> {
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    fn row(version: &str, sha: &str) -> InstalledPlugin {
        InstalledPlugin {
            id: PLUGIN_ID.to_string(),
            version: version.to_string(),
            manifest_toml: manifest_toml(PLUGIN_ID, version),
            artifact_sha256: sha.to_string(),
            enabled: true,
            disabled_reason: None,
            db_role_password: None,
            installed_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn host() -> PluginHost {
        PluginHost::new(host_version(), DEFAULT_MAX_FAILURES, DEFAULT_CALL_DEADLINE)
    }

    /// The baseline the rest of these tests ablate: an installed plugin
    /// whose artifact is on disk and matches its digest is registered, and
    /// after that the host will answer for it.
    ///
    /// "Registered", not "reachable" - nothing in this build dispatches to a
    /// plugin yet. What this pins is that the host knows about it at all,
    /// which is what no running server managed before.
    #[tokio::test]
    async fn an_installed_plugin_is_registered_with_the_host() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = PluginArtifacts::new(dir.path());
        let id = PluginId::new(PLUGIN_ID).unwrap();
        let sha = artifacts.write(&id, "0.1.0", &echo_module()).unwrap();

        let store = FakeStore::with_row(row("0.1.0", &sha));
        let host = host();

        let report = load_installed_plugins(
            &store,
            Some(&host),
            &artifacts,
            None,
            &crate::services::plugins::DeferredIssuer::default(),
        )
        .await
        .unwrap();

        assert_eq!(report.loaded, vec![id.clone()]);
        assert!(report.failed.is_empty(), "failed: {:?}", report.failed);
        assert!(
            host.status(&id).is_some(),
            "a loaded plugin must be one the host can answer about; that is what \
             'installed' failing to survive a restart looked like before"
        );
    }

    /// A plugin whose artifact no longer matches the digest recorded at
    /// install is not loaded, is switched off for the next boot, and leaves
    /// a record saying why.
    ///
    /// Ablation: drop the `disable_after_failure` call in
    /// `register_or_disable` and the row stays enabled - the next boot
    /// retries the same bad artifact, and every boot after that.
    #[tokio::test]
    async fn a_tampered_artifact_is_not_loaded_and_is_disabled_for_next_boot() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = PluginArtifacts::new(dir.path());
        let id = PluginId::new(PLUGIN_ID).unwrap();
        let installed_sha = artifacts.write(&id, "0.1.0", &echo_module()).unwrap();

        // The file changes after install.
        std::fs::write(artifacts.path_for(&id, "0.1.0").unwrap(), b"\0asm not it").unwrap();

        let store = FakeStore::with_row(row("0.1.0", &installed_sha));
        let host = host();

        let report = load_installed_plugins(
            &store,
            Some(&host),
            &artifacts,
            None,
            &crate::services::plugins::DeferredIssuer::default(),
        )
        .await
        .unwrap();

        assert!(
            report.loaded.is_empty(),
            "a tampered artifact must not load"
        );
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].disabled);
        assert!(host.status(&id).is_none());

        let after = store.row(PLUGIN_ID);
        assert!(
            !after.enabled,
            "the plugin must be disabled for the next boot"
        );
        assert!(
            after
                .disabled_reason
                .as_deref()
                .is_some_and(|r| r.contains("digest")),
            "the stored reason should name the digest, got {:?}",
            after.disabled_reason
        );
        assert_eq!(store.event_kinds(), vec!["load_failed"]);
    }

    /// A plugin a previous boot disabled stays disabled.
    ///
    /// This is the BTCPay failure in one test: their crash handler disables
    /// a plugin and restarts, and the disable does not survive the restart,
    /// so the server walks straight back into the crash. Ablation: drop the
    /// `!row.enabled` check and this loads.
    #[tokio::test]
    async fn a_plugin_disabled_by_a_previous_boot_is_not_loaded_again() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = PluginArtifacts::new(dir.path());
        let id = PluginId::new(PLUGIN_ID).unwrap();
        // The artifact is perfectly good. Being disabled is the only reason
        // it must not load - otherwise this would pass for the wrong reason.
        let sha = artifacts.write(&id, "0.1.0", &echo_module()).unwrap();

        let mut disabled = row("0.1.0", &sha);
        disabled.enabled = false;
        disabled.disabled_reason = Some("trapped 3 times in a row".to_string());

        let store = FakeStore::with_row(disabled);
        let host = host();

        let report = load_installed_plugins(
            &store,
            Some(&host),
            &artifacts,
            None,
            &crate::services::plugins::DeferredIssuer::default(),
        )
        .await
        .unwrap();

        assert!(report.loaded.is_empty());
        assert_eq!(
            report.skipped,
            vec![(id.clone(), Some("trapped 3 times in a row".to_string()))],
            "the reason it is off must survive the restart along with the disable"
        );
        assert!(host.status(&id).is_none());
    }

    /// Safe mode loads nothing - and, just as importantly, records nothing.
    ///
    /// A safe-mode boot that disabled plugins on its way past would make the
    /// recovery flag destructive: clearing it and restarting would come back
    /// with everything still off, and an admin would have no way to tell
    /// that from the plugins having genuinely failed.
    #[tokio::test]
    async fn safe_mode_loads_nothing_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = PluginArtifacts::new(dir.path());
        let id = PluginId::new(PLUGIN_ID).unwrap();
        let sha = artifacts.write(&id, "0.1.0", &echo_module()).unwrap();

        let store = FakeStore::with_row(row("0.1.0", &sha));

        let report = load_installed_plugins(
            &store,
            None,
            &artifacts,
            None,
            &crate::services::plugins::DeferredIssuer::default(),
        )
        .await
        .unwrap();

        assert!(report.safe_mode);
        assert!(report.loaded.is_empty());
        assert_eq!(report.skipped.len(), 1);

        let after = store.row(PLUGIN_ID);
        assert!(
            after.enabled && after.disabled_reason.is_none(),
            "safe mode must leave the install record exactly as it found it"
        );
        assert!(
            store.event_kinds().is_empty(),
            "safe mode is a property of one boot, not an event in a plugin's history"
        );
    }

    /// The row and the manifest must agree about which plugin this is. If
    /// they do not, the install record does not describe the artifact on
    /// disk and neither is safe to act on.
    #[tokio::test]
    async fn a_manifest_that_disagrees_with_its_install_record_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = PluginArtifacts::new(dir.path());
        let id = PluginId::new(PLUGIN_ID).unwrap();
        let sha = artifacts.write(&id, "0.1.0", &echo_module()).unwrap();

        let mut mismatched = row("0.1.0", &sha);
        mismatched.manifest_toml = manifest_toml("cash.random.somethingelse", "0.1.0");

        let store = FakeStore::with_row(mismatched);
        let host = host();

        let report = load_installed_plugins(
            &store,
            Some(&host),
            &artifacts,
            None,
            &crate::services::plugins::DeferredIssuer::default(),
        )
        .await
        .unwrap();

        assert!(report.loaded.is_empty());
        assert!(
            report.failed[0].reason.contains("declares id"),
            "got {:?}",
            report.failed[0].reason
        );
        assert!(!store.row(PLUGIN_ID).enabled);
    }

    /// A version mismatch is the same class of disagreement: the row names
    /// the artifact that was installed, and a manifest claiming a different
    /// version means one of the two is stale.
    #[tokio::test]
    async fn a_manifest_version_that_disagrees_with_the_record_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = PluginArtifacts::new(dir.path());
        let id = PluginId::new(PLUGIN_ID).unwrap();
        let sha = artifacts.write(&id, "0.1.0", &echo_module()).unwrap();

        let mut mismatched = row("0.1.0", &sha);
        mismatched.manifest_toml = manifest_toml(PLUGIN_ID, "0.2.0");

        let store = FakeStore::with_row(mismatched);
        let host = host();

        let report = load_installed_plugins(
            &store,
            Some(&host),
            &artifacts,
            None,
            &crate::services::plugins::DeferredIssuer::default(),
        )
        .await
        .unwrap();

        assert!(report.loaded.is_empty());
        assert!(
            report.failed[0].reason.contains("declares version"),
            "got {:?}",
            report.failed[0].reason
        );
    }

    /// A missing artifact does not take the boot down - and does not
    /// permanently disable the plugin either.
    ///
    /// This is the case that separates a broken plugin from a broken
    /// machine. A container that restarts before its plugin volume is
    /// attached sees every artifact missing; disabling them all would mean
    /// attaching the volume does not bring anything back, and with no
    /// re-enable endpoint in this slice the only recovery would be editing
    /// the database by hand - the exact outcome this module exists to
    /// avoid.
    ///
    /// Ablation: classify `NotFound` as `FailureKind::Plugin` and this goes
    /// red on the `enabled` assertion.
    #[tokio::test]
    async fn a_missing_artifact_leaves_the_plugin_enabled_for_the_next_boot() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = PluginArtifacts::new(dir.path());

        let store = FakeStore::with_row(row("0.1.0", &payserver_plugin_host::digest(b"gone")));
        let host = host();

        let report = load_installed_plugins(
            &store,
            Some(&host),
            &artifacts,
            None,
            &crate::services::plugins::DeferredIssuer::default(),
        )
        .await
        .expect("a missing artifact is not a boot failure");

        assert_eq!(report.failed.len(), 1);
        assert!(
            !report.failed[0].disabled,
            "a file that is missing today may be present tomorrow"
        );

        let after = store.row(PLUGIN_ID);
        assert!(
            after.enabled && after.disabled_reason.is_none(),
            "an unreachable artifact must not switch the plugin off; got enabled={} \
             reason={:?}",
            after.enabled,
            after.disabled_reason
        );
        assert_eq!(
            store.event_kinds(),
            vec!["load_failed"],
            "it is still worth recording that the boot could not load it"
        );
    }

    /// The whole plugin set surviving an unmounted volume, which is the
    /// scenario the split exists for: three plugins, no artifact directory,
    /// every one of them still enabled afterwards.
    #[tokio::test]
    async fn an_unmounted_artifact_volume_does_not_disable_everything_installed() {
        let artifacts = PluginArtifacts::new("/definitely/not/mounted");
        let sha = payserver_plugin_host::digest(b"whatever");

        let mut rows = Vec::new();
        for n in 0..3 {
            let mut r = row("0.1.0", &sha);
            r.id = format!("cash.random.plugin{n}");
            r.manifest_toml = manifest_toml(&r.id, "0.1.0");
            rows.push(r);
        }
        let store = FakeStore {
            rows: Mutex::new(rows),
            events: Mutex::new(Vec::new()),
        };
        let host = host();

        let report = load_installed_plugins(
            &store,
            Some(&host),
            &artifacts,
            None,
            &crate::services::plugins::DeferredIssuer::default(),
        )
        .await
        .unwrap();

        assert_eq!(report.failed.len(), 3);
        assert!(report.failed.iter().all(|f| !f.disabled));
        assert!(
            store.rows.lock().unwrap().iter().all(|r| r.enabled),
            "mounting the volume and restarting must be enough to recover"
        );
    }

    /// The other side of the split: a digest mismatch will never start
    /// matching, so retrying it forever is the wrong answer and it is
    /// switched off.
    #[tokio::test]
    async fn a_failure_that_retrying_cannot_fix_is_persisted_but_one_that_can_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = PluginArtifacts::new(dir.path());
        let id = PluginId::new(PLUGIN_ID).unwrap();
        let sha = artifacts.write(&id, "0.1.0", &echo_module()).unwrap();
        std::fs::write(artifacts.path_for(&id, "0.1.0").unwrap(), b"tampered").unwrap();

        let store = FakeStore::with_row(row("0.1.0", &sha));
        let host = host();

        let report = load_installed_plugins(
            &store,
            Some(&host),
            &artifacts,
            None,
            &crate::services::plugins::DeferredIssuer::default(),
        )
        .await
        .unwrap();

        assert!(report.failed[0].disabled);
        assert!(!store.row(PLUGIN_ID).enabled);
    }
}
