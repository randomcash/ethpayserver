//! The load-time gate: which manifests this host accepts before any plugin
//! code runs.

use std::collections::HashMap;

use payserver_plugin_api::{Manifest, PluginId, Version};

use super::error::PluginLoadError;

/// The name a plugin manifest uses to depend on this server, e.g.
/// `ethpayserver:^1.2.0`. The same `name:requirement` mechanism plugins use
/// to depend on each other.
const HOST_NAME: &str = "ethpayserver";

/// This build's version, as presented to plugins for `ethpayserver`
/// dependency matching.
#[allow(clippy::expect_used)]
pub fn host_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("CARGO_PKG_VERSION is valid semver")
}

/// The manifests this host has accepted, keyed by [`PluginId`].
///
/// This is the load-time gate only: no wasmtime, no instantiation, no
/// dispatch. Registering a manifest just means the host has decided it is
/// safe to *consider* loading — everything after that is out of scope here.
#[derive(Debug)]
pub struct PluginRegistry {
    host_version: Version,
    loaded: HashMap<PluginId, Manifest>,
    /// Safe mode: every manifest is accepted without complaint but never
    /// registered, so `loaded` stays empty no matter what is offered to it.
    /// Set once at construction from boot config - see `Config::safe_mode`.
    safe_mode: bool,
}

impl PluginRegistry {
    pub fn new(host_version: Version) -> Self {
        Self {
            host_version,
            loaded: HashMap::new(),
            safe_mode: false,
        }
    }

    /// Puts the registry in safe mode: every future [`register`](Self::register)
    /// call is a no-op success, so nothing ever loads. Does not affect
    /// manifests already registered before this call.
    #[must_use]
    pub fn with_safe_mode(mut self, safe_mode: bool) -> Self {
        self.safe_mode = safe_mode;
        self
    }

    /// Validates `manifest` and, if it passes, registers it.
    ///
    /// In safe mode, returns `Ok(())` immediately without validating or
    /// registering - safe mode disables plugins, it does not report them as
    /// broken.
    ///
    /// Otherwise refused rather than silently accepted when:
    /// - the manifest declares no `ethpayserver` dependency at all;
    /// - its `ethpayserver` requirement does not match this host's version —
    ///   the error names both the requirement and the host version, since a
    ///   refusal that does not say which version was wanted is the failure
    ///   mode admins actually hit;
    /// - its id is already registered — the second manifest is refused, not
    ///   merged over the first.
    pub fn register(&mut self, manifest: Manifest) -> Result<(), PluginLoadError> {
        if self.safe_mode {
            return Ok(());
        }

        let dependency = manifest.dependency(HOST_NAME).ok_or_else(|| {
            PluginLoadError::MissingHostDependency {
                plugin_id: manifest.id.clone(),
            }
        })?;

        if !dependency.is_satisfied_by(&self.host_version) {
            return Err(PluginLoadError::IncompatibleHost {
                plugin_id: manifest.id.clone(),
                requirement: dependency.requirement.clone(),
                host_version: self.host_version.clone(),
            });
        }

        if self.loaded.contains_key(&manifest.id) {
            return Err(PluginLoadError::DuplicateId(manifest.id.clone()));
        }

        self.loaded.insert(manifest.id.clone(), manifest);
        Ok(())
    }

    /// A previously registered manifest, by id.
    pub fn get(&self, id: &PluginId) -> Option<&Manifest> {
        self.loaded.get(id)
    }

    pub fn len(&self) -> usize {
        self.loaded.len()
    }

    pub fn is_empty(&self) -> bool {
        self.loaded.is_empty()
    }

    /// Whether this registry is refusing to register anything.
    pub fn safe_mode(&self) -> bool {
        self.safe_mode
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn manifest(toml: &str) -> Manifest {
        toml.parse().unwrap()
    }

    fn host(version: &str) -> PluginRegistry {
        PluginRegistry::new(Version::parse(version).unwrap())
    }

    /// Ticket test 1: a manifest whose `dependencies` the host does not
    /// satisfy is refused, and the error names both the requirement and the
    /// host version.
    #[test]
    fn refuses_a_manifest_the_host_does_not_satisfy() {
        let mut registry = host("2.0.0");
        let manifest = manifest(
            r#"
                id = "cash.random.billing"
                version = "0.1.0"
                dependencies = ["ethpayserver:^1.2.0"]
                kind = "action"
            "#,
        );

        let err = registry.register(manifest).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("^1.2.0"),
            "error should name the requirement: {message}"
        );
        assert!(
            message.contains("2.0.0"),
            "error should name the host version: {message}"
        );
        assert!(registry.is_empty());
    }

    #[test]
    fn accepts_a_manifest_the_host_satisfies() {
        let mut registry = host("1.2.5");
        let manifest = manifest(
            r#"
                id = "cash.random.billing"
                version = "0.1.0"
                dependencies = ["ethpayserver:^1.2.0"]
                kind = "action"
            "#,
        );

        registry.register(manifest).unwrap();
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn refuses_a_manifest_with_no_host_dependency() {
        let mut registry = host("1.2.5");
        let manifest = manifest(
            r#"
                id = "cash.random.billing"
                version = "0.1.0"
                dependencies = []
                kind = "action"
            "#,
        );

        assert_eq!(
            registry.register(manifest).unwrap_err(),
            PluginLoadError::MissingHostDependency {
                plugin_id: PluginId::new("cash.random.billing").unwrap()
            }
        );
    }

    /// Ticket test 4: two plugins declaring the same `id` — the second is
    /// refused, not silently shadowing the first.
    #[test]
    fn refuses_a_duplicate_id() {
        let mut registry = host("1.2.5");
        let first = manifest(
            r#"
                id = "cash.random.billing"
                version = "0.1.0"
                dependencies = ["ethpayserver:^1.2.0"]
                kind = "action"
            "#,
        );
        let second = manifest(
            r#"
                id = "cash.random.billing"
                version = "9.9.9"
                dependencies = ["ethpayserver:^1.2.0"]
                kind = "filter"
            "#,
        );

        registry.register(first.clone()).unwrap();
        let err = registry.register(second).unwrap_err();

        assert!(matches!(err, PluginLoadError::DuplicateId(_)));
        assert_eq!(registry.len(), 1);
        // The first registration is untouched, not overwritten by the refused one.
        assert_eq!(registry.get(&first.id), Some(&first));
    }

    /// In safe mode, a manifest that would otherwise register just fine is
    /// accepted-and-dropped, not loaded.
    #[test]
    fn safe_mode_accepts_without_loading() {
        let mut registry = host("1.2.5").with_safe_mode(true);
        let manifest = manifest(
            r#"
                id = "cash.random.billing"
                version = "0.1.0"
                dependencies = ["ethpayserver:^1.2.0"]
                kind = "action"
            "#,
        );

        registry.register(manifest).unwrap();
        assert!(registry.is_empty());
        assert!(registry.safe_mode());
    }

    /// Safe mode does not turn an invalid manifest into an error either - it
    /// disables plugins, it does not report on them.
    #[test]
    fn safe_mode_accepts_even_a_manifest_that_would_otherwise_be_refused() {
        let mut registry = host("2.0.0").with_safe_mode(true);
        let manifest = manifest(
            r#"
                id = "cash.random.billing"
                version = "0.1.0"
                dependencies = ["ethpayserver:^1.2.0"]
                kind = "action"
            "#,
        );

        assert!(registry.register(manifest).is_ok());
        assert!(registry.is_empty());
    }

    /// Clearing safe mode on the same registry that had it set restores
    /// normal registration - nothing about the earlier safe-mode boot
    /// lingers, since the registry never persisted anything to disk.
    #[test]
    fn clearing_safe_mode_restores_registration() {
        let mut registry = host("1.2.5").with_safe_mode(true);
        let manifest = manifest(
            r#"
                id = "cash.random.billing"
                version = "0.1.0"
                dependencies = ["ethpayserver:^1.2.0"]
                kind = "action"
            "#,
        );

        registry.register(manifest.clone()).unwrap();
        assert!(registry.is_empty(), "safe mode should have dropped it");

        let mut registry = registry.with_safe_mode(false);
        registry.register(manifest).unwrap();
        assert_eq!(registry.len(), 1);
    }
}
