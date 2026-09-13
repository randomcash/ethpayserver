//! Runs plugins: dispatches actions and filters, bounds every call with a
//! deadline, and disables a plugin after repeated failure — with the reason
//! visible without going anywhere near wasmtime.
//!
//! # Action vs. filter
//!
//! These are two different entry points, not one dispatch call with a flag,
//! because they fail differently:
//!
//! - [`PluginHost::run_action`] is fire-and-forget. It returns nothing, so
//!   there is nothing for a failing action to change; failure is only ever
//!   observable via [`PluginHost::status`].
//! - [`PluginHost::run_filter`] carries the call's deadline and, when the
//!   filter cannot run, resolves to the manifest's declared
//!   [`FailureMode`] — closed unless the manifest said otherwise, per
//!   [`payserver_plugin_api::Manifest`]'s own parse-time default.
//!
//! Neither of these is wired to anything that touches payment detection,
//! crediting or confirmation, and nothing in this module could be: the only
//! hook points that exist are the generic `run_action`/`run_filter` calls
//! above, keyed by a plugin-supplied export name chosen by whoever adds the
//! next call site. The money-path hook points this must never become
//! belong to a future, host-API-surface ticket — this slice adds no hook
//! points of its own at all.
//!
//! # Why the admin surface survives a crash loop
//!
//! [`PluginHost::status`] never touches the [`std::sync::Mutex`] guarding a
//! plugin's wasmtime instance. It reads two atomics and, at most, a small
//! `RwLock<String>` holding the disabled reason — none of which a stuck or
//! looping call ever holds while blocked inside wasm. And once a plugin is
//! disabled, `run_action`/`run_filter` stop calling into wasmtime for it at
//! all, so a crash loop is bounded to `max_failures` attempts, not
//! unbounded.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use payserver_plugin_api::{FailureMode, Manifest, PluginId, Version};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::PluginLoadError;
use super::registry::PluginRegistry;
use super::runtime::{PluginEngine, PluginInstance, PluginWasmError};

/// A plugin failed either the load-time gate or wasm compilation /
/// instantiation.
#[derive(Debug, thiserror::Error)]
pub enum PluginHostError {
    #[error(transparent)]
    Rejected(#[from] PluginLoadError),
    #[error(transparent)]
    Wasm(#[from] PluginWasmError),
}

/// What happened when a filter could not be run to completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterOutcome<T> {
    /// The filter ran and answered.
    Ran(T),
    /// The filter could not run — missing, disabled, trapped, timed out, or
    /// its answer didn't parse. `allowed` is the manifest's
    /// [`FailureMode`] applied: `true` for [`FailureMode::Open`], `false`
    /// for [`FailureMode::Closed`] (the default for a filter that declares
    /// nothing). `reason` is always populated, for the admin's benefit.
    CouldNotRun { allowed: bool, reason: String },
}

impl<T> FilterOutcome<T> {
    fn could_not_run(failure_mode: FailureMode, reason: impl Into<String>) -> Self {
        Self::CouldNotRun {
            allowed: matches!(failure_mode, FailureMode::Open),
            reason: reason.into(),
        }
    }
}

/// A point-in-time read of one plugin's health, safe to expose on an admin
/// page regardless of what the plugin itself is doing right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginStatusSnapshot {
    pub id: PluginId,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    pub consecutive_failures: u32,
}

enum EntryStatus {
    Enabled,
    Disabled { reason: String },
}

/// One registered, instantiated plugin plus the bookkeeping needed to
/// disable it and to report on it without touching its execution lock.
///
/// The manifest itself lives in [`PluginRegistry`], not here — this only
/// needs `max_failures`, resolved once at registration time.
struct PluginEntry {
    instance: Mutex<PluginInstance>,
    failures: AtomicU32,
    status: RwLock<EntryStatus>,
    max_failures: u32,
}

impl PluginEntry {
    fn new(instance: PluginInstance, max_failures: u32) -> Self {
        Self {
            instance: Mutex::new(instance),
            failures: AtomicU32::new(0),
            status: RwLock::new(EntryStatus::Enabled),
            max_failures,
        }
    }

    fn is_enabled(&self) -> bool {
        matches!(
            *self.status.read().unwrap_or_else(PoisonError::into_inner),
            EntryStatus::Enabled
        )
    }

    /// Records a successful call, resetting the failure streak — an
    /// occasional failure in an otherwise-healthy plugin should not creep
    /// it towards disablement.
    fn record_success(&self) {
        self.failures.store(0, Ordering::SeqCst);
    }

    /// Records a failed call. Once `max_failures` consecutive failures have
    /// been seen, disables the plugin and remembers why — the reason an
    /// admin needs to see to fix it, not just "disabled".
    fn record_failure(&self, reason: String) {
        let failures = self.failures.fetch_add(1, Ordering::SeqCst) + 1;
        if failures >= self.max_failures {
            *self.status.write().unwrap_or_else(PoisonError::into_inner) =
                EntryStatus::Disabled { reason };
        }
    }

    fn snapshot(&self, id: PluginId) -> PluginStatusSnapshot {
        let consecutive_failures = self.failures.load(Ordering::SeqCst);
        let disabled_reason = match &*self.status.read().unwrap_or_else(PoisonError::into_inner) {
            EntryStatus::Enabled => None,
            EntryStatus::Disabled { reason } => Some(reason.clone()),
        };
        PluginStatusSnapshot {
            id,
            enabled: disabled_reason.is_none(),
            disabled_reason,
            consecutive_failures,
        }
    }
}

/// Loads, runs and bounds plugins.
///
/// One [`PluginEngine`] (and its epoch ticker) is shared across every
/// plugin this host loads; each plugin gets its own instantiated, kept-alive
/// [`PluginInstance`] behind its own call lock, so a slow or stuck plugin
/// never blocks another plugin's calls or a status read.
///
/// The engine is behind an [`Arc`], not owned outright, because
/// [`PluginHost::run_action`] detaches its call onto a background task that
/// can outlive `&self` — without its own strong reference to the engine, a
/// `PluginHost` dropped while an action is still in flight would take the
/// epoch ticker down with it, and a plugin stuck in a loop would then have
/// nothing left to interrupt it: the deadline would never fire.
pub struct PluginHost {
    engine: Arc<PluginEngine>,
    registry: RwLock<PluginRegistry>,
    entries: RwLock<HashMap<PluginId, Arc<PluginEntry>>>,
    max_failures: u32,
    call_deadline: Duration,
}

impl PluginHost {
    pub fn new(host_version: Version, max_failures: u32, call_deadline: Duration) -> Self {
        Self {
            engine: Arc::new(PluginEngine::new()),
            registry: RwLock::new(PluginRegistry::new(host_version)),
            entries: RwLock::new(HashMap::new()),
            max_failures,
            call_deadline,
        }
    }

    #[cfg(test)]
    fn with_tick(
        host_version: Version,
        max_failures: u32,
        call_deadline: Duration,
        tick: Duration,
    ) -> Self {
        Self {
            engine: Arc::new(PluginEngine::with_tick(tick)),
            registry: RwLock::new(PluginRegistry::new(host_version)),
            entries: RwLock::new(HashMap::new()),
            max_failures,
            call_deadline,
        }
    }

    /// Validates `manifest` against the load-time gate, then compiles and
    /// instantiates `wasm` — once; the resulting instance is kept and reused
    /// across every future call.
    pub fn register(&self, manifest: Manifest, wasm: &[u8]) -> Result<(), PluginHostError> {
        let id = manifest.id.clone();
        self.registry
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .register(manifest.clone())?;

        let module = self.engine.compile(wasm)?;
        let instance = self.engine.instantiate(&module)?;
        let entry = Arc::new(PluginEntry::new(instance, self.max_failures));
        self.entries
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, entry);
        Ok(())
    }

    /// A point-in-time health read for one plugin. Never touches the
    /// plugin's execution lock — see the module-level doc comment on why
    /// that makes this safe to call while the plugin is crash-looping.
    pub fn status(&self, id: &PluginId) -> Option<PluginStatusSnapshot> {
        let entries = self.entries.read().unwrap_or_else(PoisonError::into_inner);
        entries.get(id).map(|entry| entry.snapshot(id.clone()))
    }

    fn enabled_entry(&self, id: &PluginId) -> Option<Arc<PluginEntry>> {
        let entries = self.entries.read().unwrap_or_else(PoisonError::into_inner);
        let entry = entries.get(id)?;
        entry.is_enabled().then(|| entry.clone())
    }

    /// Fires `export` on plugin `id` and does not wait for it. Failure —
    /// missing plugin, trap, timeout, an answer that can't be read — is
    /// folded into that plugin's failure count and is otherwise invisible to
    /// the caller: an action can never block a request or change its
    /// outcome, by construction, since this returns nothing to change it
    /// with.
    pub fn run_action<Req: Serialize>(&self, id: &PluginId, export: &str, req: &Req) {
        let Some(entry) = self.enabled_entry(id) else {
            return;
        };
        let arg = match serde_json::to_vec(req) {
            Ok(arg) => arg,
            Err(_) => return,
        };
        let export = export.to_string();
        let ticks = self.engine.ticks_for(self.call_deadline);
        // Held for the lifetime of the spawned task, not just this method —
        // see the struct doc on `PluginHost::engine` for why a detached call
        // needs its own claim on the engine that enforces its deadline.
        let engine = Arc::clone(&self.engine);

        tokio::spawn(async move {
            let outcome = tokio::task::spawn_blocking(move || {
                let mut instance = entry_instance_lock(&entry);
                let result = instance.call_raw(&export, &arg, ticks);
                drop(instance);
                drop(engine);
                (entry, result)
            })
            .await;

            match outcome {
                Ok((entry, Ok(_))) => entry.record_success(),
                Ok((entry, Err(err))) => entry.record_failure(err.to_string()),
                Err(_join_err) => {
                    // The blocking task itself panicked; we no longer have
                    // the `Arc<PluginEntry>` (it was moved into the aborted
                    // task), so there is nothing left to record against.
                }
            }
        });
    }

    /// Runs `export` on plugin `id` as a filter, bounded by this host's
    /// call deadline, and resolves to [`FilterOutcome::Ran`] on success or
    /// [`FilterOutcome::CouldNotRun`] — applying the manifest's
    /// [`FailureMode`] — on any failure, including the plugin not existing.
    pub async fn run_filter<Req, Resp>(
        &self,
        id: &PluginId,
        export: &str,
        req: &Req,
    ) -> FilterOutcome<Resp>
    where
        Req: Serialize,
        Resp: DeserializeOwned + Send + 'static,
    {
        let failure_mode = self.failure_mode_for(id);
        let Some(entry) = self.enabled_entry(id) else {
            return FilterOutcome::could_not_run(
                failure_mode,
                format!("plugin {id} is not available to run"),
            );
        };
        let arg = match serde_json::to_vec(req) {
            Ok(arg) => arg,
            Err(e) => {
                return FilterOutcome::could_not_run(
                    failure_mode,
                    format!("could not serialise filter argument: {e}"),
                );
            }
        };
        let export = export.to_string();
        let ticks = self.engine.ticks_for(self.call_deadline);

        let outcome = tokio::task::spawn_blocking(move || {
            let mut instance = entry_instance_lock(&entry);
            let result = instance.call_raw(&export, &arg, ticks);
            drop(instance);
            (entry, result)
        })
        .await;

        match outcome {
            Ok((entry, Ok(bytes))) => match serde_json::from_slice::<Resp>(&bytes) {
                Ok(resp) => {
                    entry.record_success();
                    FilterOutcome::Ran(resp)
                }
                Err(e) => {
                    let reason = format!("filter answer did not parse: {e}");
                    entry.record_failure(reason.clone());
                    FilterOutcome::could_not_run(failure_mode, reason)
                }
            },
            Ok((entry, Err(call_err))) => {
                let reason = call_err.to_string();
                entry.record_failure(reason.clone());
                FilterOutcome::could_not_run(failure_mode, reason)
            }
            Err(join_err) => FilterOutcome::could_not_run(
                failure_mode,
                format!("filter task did not complete: {join_err}"),
            ),
        }
    }

    fn failure_mode_for(&self, id: &PluginId) -> FailureMode {
        self.registry
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .and_then(|manifest| manifest.failure_mode)
            .unwrap_or(FailureMode::Closed)
    }
}

fn entry_instance_lock(entry: &PluginEntry) -> std::sync::MutexGuard<'_, PluginInstance> {
    entry
        .instance
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::time::{Duration, Instant};

    use serde::{Deserialize, Serialize};

    use super::super::runtime::fixtures;
    use super::*;

    #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
    struct Ping {
        n: u32,
    }

    fn host(max_failures: u32) -> PluginHost {
        PluginHost::with_tick(
            Version::new(1, 0, 0),
            max_failures,
            Duration::from_millis(20),
            Duration::from_millis(5),
        )
    }

    fn manifest(id: &str, kind: &str, failure_mode: Option<&str>) -> Manifest {
        let failure_mode_line = failure_mode
            .map(|m| format!("failure_mode = {m:?}\n"))
            .unwrap_or_default();
        format!(
            r#"
                id = {id:?}
                version = "0.1.0"
                dependencies = ["ethpayserver:^1.0.0"]
                kind = {kind:?}
                {failure_mode_line}
            "#
        )
        .parse()
        .unwrap()
    }

    /// Ticket test 1: a panicking plugin, called repeatedly, ends up
    /// disabled with a reason an admin can read — and every one of those
    /// calls got a clean answer back, not a panic.
    #[tokio::test]
    async fn panicking_plugin_is_disabled_with_a_visible_reason() {
        let host = host(3);
        let id = PluginId::new("cash.random.panics").unwrap();
        host.register(
            manifest("cash.random.panics", "action", None),
            &fixtures::panicking_module(),
        )
        .unwrap();

        for _ in 0..3 {
            host.run_action(&id, "call", &Ping { n: 1 });
        }
        // `run_action` is fire-and-forget; give the spawned tasks a moment
        // to actually run against the (fast, local) plugin.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let status = host.status(&id).unwrap();
        assert!(!status.enabled, "expected the plugin to be disabled");
        let reason = status.disabled_reason.unwrap();
        assert!(
            reason.to_lowercase().contains("trap"),
            "reason should say it trapped: {reason}"
        );
    }

    /// Ticket test 2: a filter that never returns is interrupted by its
    /// deadline, the request completes (this test itself does), and the
    /// plugin ends up disabled.
    #[tokio::test]
    async fn deadline_disables_the_plugin_and_the_request_completes() {
        let host = host(1);
        let id = PluginId::new("cash.random.hangs").unwrap();
        host.register(
            manifest("cash.random.hangs", "filter", None),
            &fixtures::infinite_loop_module(),
        )
        .unwrap();

        let started = Instant::now();
        let outcome = host
            .run_filter::<_, Ping>(&id, "call", &Ping { n: 1 })
            .await;
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_secs(5),
            "took {elapsed:?} to come back"
        );
        assert!(
            matches!(outcome, FilterOutcome::CouldNotRun { allowed: false, .. }),
            "got {outcome:?}"
        );
        assert!(!host.status(&id).unwrap().enabled);
    }

    /// Ticket test 3: a plugin returning bytes that don't deserialise fails
    /// the filter (closed, since no failure mode was declared) instead of
    /// panicking the host.
    #[tokio::test]
    async fn garbage_answer_refuses_the_filter_without_panicking() {
        let host = host(5);
        let id = PluginId::new("cash.random.garbage").unwrap();
        host.register(
            manifest("cash.random.garbage", "filter", None),
            &fixtures::garbage_module(),
        )
        .unwrap();

        let outcome = host
            .run_filter::<_, Ping>(&id, "call", &Ping { n: 1 })
            .await;

        assert!(
            matches!(outcome, FilterOutcome::CouldNotRun { allowed: false, .. }),
            "got {outcome:?}"
        );
    }

    /// Ticket test 4: a filter that fails with no declared failure mode is
    /// refused, not allowed — the manifest's own parse-time default
    /// (`FailureMode::Closed`) must actually be what the host applies.
    #[tokio::test]
    async fn filter_with_no_declared_failure_mode_fails_closed() {
        let host = host(5);
        let id = PluginId::new("cash.random.strictfilter").unwrap();
        host.register(
            manifest("cash.random.strictfilter", "filter", None),
            &fixtures::panicking_module(),
        )
        .unwrap();

        let outcome = host
            .run_filter::<_, Ping>(&id, "call", &Ping { n: 1 })
            .await;

        assert!(
            matches!(outcome, FilterOutcome::CouldNotRun { allowed: false, .. }),
            "a filter with no declared failure_mode must fail closed, got {outcome:?}"
        );
    }

    /// The contrasting case: a filter that declares `failure_mode = "open"`
    /// proceeds when it fails, proving the host actually reads the
    /// manifest's choice rather than hardcoding closed.
    #[tokio::test]
    async fn filter_with_open_failure_mode_proceeds_on_failure() {
        let host = host(5);
        let id = PluginId::new("cash.random.openfilter").unwrap();
        host.register(
            manifest("cash.random.openfilter", "filter", Some("open")),
            &fixtures::panicking_module(),
        )
        .unwrap();

        let outcome = host
            .run_filter::<_, Ping>(&id, "call", &Ping { n: 1 })
            .await;

        assert!(
            matches!(outcome, FilterOutcome::CouldNotRun { allowed: true, .. }),
            "got {outcome:?}"
        );
    }

    /// Ticket test 5: while a plugin is crash-looping (or simply mid-call),
    /// a status read — the admin page's data source — stays fast. It never
    /// waits on the same lock a stuck call is holding.
    #[tokio::test]
    async fn status_reads_stay_fast_while_a_plugin_is_stuck() {
        let host = Arc::new(host(100));
        let id = PluginId::new("cash.random.stuck").unwrap();
        host.register(
            manifest("cash.random.stuck", "action", None),
            &fixtures::infinite_loop_module(),
        )
        .unwrap();

        // Kick off a call that will occupy the plugin's instance lock for
        // roughly its whole (comparatively long) deadline.
        let in_flight = host.clone();
        let in_flight_id = id.clone();
        tokio::spawn(async move {
            in_flight.run_action(&in_flight_id, "call", &Ping { n: 1 });
        });
        tokio::time::sleep(Duration::from_millis(10)).await;

        let started = Instant::now();
        let status = tokio::time::timeout(Duration::from_millis(50), async { host.status(&id) })
            .await
            .unwrap();
        assert!(status.is_some());
        assert!(
            started.elapsed() < Duration::from_millis(50),
            "status() took {:?}",
            started.elapsed()
        );
    }

    /// Once disabled, the host stops calling into wasmtime for that plugin
    /// at all — the crash loop is bounded to `max_failures`, not unbounded.
    #[tokio::test]
    async fn a_disabled_plugin_is_never_called_again() {
        let host = host(2);
        let id = PluginId::new("cash.random.diesquick").unwrap();
        host.register(
            manifest("cash.random.diesquick", "action", None),
            &fixtures::panicking_module(),
        )
        .unwrap();

        for _ in 0..2 {
            host.run_action(&id, "call", &Ping { n: 1 });
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!host.status(&id).unwrap().enabled);

        for _ in 0..10 {
            host.run_action(&id, "call", &Ping { n: 1 });
        }
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Failures stop accumulating past disablement: nothing after
        // disablement ever reached wasmtime to fail again.
        assert_eq!(host.status(&id).unwrap().consecutive_failures, 2);
    }

    #[test]
    fn register_rejects_what_the_load_time_gate_rejects() {
        let host = host(3);
        // Dependency requirement the 1.0.0 test host does not satisfy.
        let bad: Manifest = r#"
            id = "cash.random.old"
            version = "0.1.0"
            dependencies = ["ethpayserver:^9.9.9"]
            kind = "action"
        "#
        .parse()
        .unwrap();

        let err = host.register(bad, &fixtures::echo_module()).unwrap_err();
        assert!(matches!(err, PluginHostError::Rejected(_)));
    }
}
