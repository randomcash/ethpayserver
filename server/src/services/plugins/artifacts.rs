//! Where a plugin's wasm lives on disk, and what makes it the same wasm that
//! was installed.
//!
//! A plugin is code this server compiles and runs. The artifact directory is
//! writable by whoever administers the box, and on a containerised install it
//! is a mounted volume - so "the file at the path we recorded" and "the file
//! we accepted at install" are not the same claim. The digest is what makes
//! them one: it is taken over the bytes at install, stored next to the
//! install record, and re-checked before the module is handed to wasmtime on
//! every boot.
//!
//! That check is not a security boundary against an admin with disk access -
//! nothing here could be. It is what turns a swapped, truncated or
//! half-written artifact into a refusal with a reason instead of arbitrary
//! code, which is the failure mode that actually happens: an interrupted
//! copy, a restored volume snapshot, a half-finished manual upgrade.

use std::path::{Path, PathBuf};

use payserver_plugin_api::PluginId;
use sha2::{Digest, Sha256};

/// Why an artifact could not be produced.
#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("plugin artifact not found at {0}")]
    NotFound(PathBuf),

    #[error("could not read plugin artifact at {path}: {source}")]
    Unreadable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Kept separate from [`ArtifactError::Unreadable`] because the two
    /// describe opposite operations and the distinction is the whole
    /// message: a read-only mount or a full volume reported as "could not
    /// read" sends an admin looking at the wrong thing.
    #[error("could not write plugin artifact at {path}: {source}")]
    Unwritable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The file on disk is not the file that was installed.
    #[error(
        "plugin artifact at {path} does not match the digest recorded at install \
         (expected {expected}, found {found}); refusing to run it"
    )]
    DigestMismatch {
        path: PathBuf,
        expected: String,
        found: String,
    },

    /// A version string that would escape the plugin's own directory.
    ///
    /// `semver::Version` cannot produce one, so this is unreachable through
    /// the manifest path it is actually reached by. It is checked anyway
    /// because the alternative failure is writing outside the artifact
    /// directory, and a cheap assertion is worth more than the argument that
    /// it cannot happen.
    #[error("plugin version {0:?} is not usable as a filename")]
    UnsafeVersion(String),
}

/// Lowercase hex SHA-256 of `bytes`.
#[must_use]
pub fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The directory tree holding installed plugins' wasm.
///
/// Layout is `<root>/<plugin id>/<version>.wasm`. The version is in the
/// filename rather than the directory so an upgrade adds a file instead of
/// overwriting the only copy of a build that currently works - the artifact
/// for the version still recorded in the database stays on disk and readable
/// through a rollback.
#[derive(Debug, Clone)]
pub struct PluginArtifacts {
    root: PathBuf,
}

impl PluginArtifacts {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The path `id` version `version` would occupy.
    ///
    /// `PluginId` is already validated to contain no path separator and no
    /// empty label, which is what makes it safe as a directory name; see
    /// `payserver_plugin_api::PluginId`.
    pub fn path_for(&self, id: &PluginId, version: &str) -> Result<PathBuf, ArtifactError> {
        if version.is_empty()
            || version.contains('/')
            || version.contains('\\')
            || version.contains("..")
        {
            return Err(ArtifactError::UnsafeVersion(version.to_string()));
        }
        Ok(self.root.join(id.as_str()).join(format!("{version}.wasm")))
    }

    /// Read the artifact and prove it is the one that was installed.
    ///
    /// The digest is compared before the bytes are returned, so a caller
    /// cannot accidentally use an unverified module by ignoring a second
    /// return value.
    pub fn read_verified(
        &self,
        id: &PluginId,
        version: &str,
        expected_sha256: &str,
    ) -> Result<Vec<u8>, ArtifactError> {
        let path = self.path_for(id, version)?;
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Err(ArtifactError::NotFound(path));
            }
            Err(source) => return Err(ArtifactError::Unreadable { path, source }),
        };

        let found = digest(&bytes);
        // Compared case-insensitively: the digest is hex, and an install
        // path that happened to store it uppercase would otherwise make
        // every subsequent boot look like tampering.
        if !found.eq_ignore_ascii_case(expected_sha256) {
            return Err(ArtifactError::DigestMismatch {
                path,
                expected: expected_sha256.to_string(),
                found,
            });
        }
        Ok(bytes)
    }

    /// Write `wasm` as `id`'s artifact for `version`, returning its digest.
    ///
    /// Writes to a temporary file in the same directory and renames it into
    /// place: a rename within one filesystem is atomic, so an interrupted
    /// install leaves either the old artifact or the new one, never a
    /// truncated file that the digest check would then have to catch on a
    /// boot the admin did not connect to the install.
    pub fn write(
        &self,
        id: &PluginId,
        version: &str,
        wasm: &[u8],
    ) -> Result<String, ArtifactError> {
        let path = self.path_for(id, version)?;
        let dir = path.parent().unwrap_or(&self.root);
        std::fs::create_dir_all(dir).map_err(|source| ArtifactError::Unreadable {
            path: dir.to_path_buf(),
            source,
        })?;

        let tmp = path.with_extension("wasm.partial");
        std::fs::write(&tmp, wasm).map_err(|source| ArtifactError::Unreadable {
            path: tmp.clone(),
            source,
        })?;
        std::fs::rename(&tmp, &path).map_err(|source| ArtifactError::Unreadable {
            path: path.clone(),
            source,
        })?;

        Ok(digest(wasm))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn id() -> PluginId {
        PluginId::new("cash.random.billing").unwrap()
    }

    #[test]
    fn writes_then_reads_back_the_same_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = PluginArtifacts::new(dir.path());

        let sha = artifacts.write(&id(), "0.1.0", b"\0asm fake").unwrap();
        let read = artifacts.read_verified(&id(), "0.1.0", &sha).unwrap();

        assert_eq!(read, b"\0asm fake");
    }

    /// An artifact that changed on disk after install is refused, rather
    /// than compiled and run.
    ///
    /// The ablation is the whole point: delete the `found != expected`
    /// comparison in `read_verified` and this returns the replacement bytes
    /// happily, which is a plugin directory that decides what code the
    /// server executes with no record that anything changed.
    #[test]
    fn an_artifact_that_changed_since_install_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = PluginArtifacts::new(dir.path());

        let installed_sha = artifacts
            .write(&id(), "0.1.0", b"the wasm that was installed")
            .unwrap();

        // Someone, or something, replaces the file.
        let path = artifacts.path_for(&id(), "0.1.0").unwrap();
        std::fs::write(&path, b"different wasm entirely").unwrap();

        let err = artifacts
            .read_verified(&id(), "0.1.0", &installed_sha)
            .unwrap_err();

        assert!(
            matches!(err, ArtifactError::DigestMismatch { .. }),
            "expected a digest mismatch, got {err:?}"
        );
    }

    /// A truncated artifact - the half-written file an interrupted copy
    /// leaves - is the same refusal, not a compile error from wasmtime.
    #[test]
    fn a_truncated_artifact_is_refused_before_it_reaches_wasmtime() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = PluginArtifacts::new(dir.path());

        let sha = artifacts
            .write(&id(), "0.1.0", b"a complete artifact")
            .unwrap();
        let path = artifacts.path_for(&id(), "0.1.0").unwrap();
        std::fs::write(&path, b"a compl").unwrap();

        assert!(matches!(
            artifacts.read_verified(&id(), "0.1.0", &sha).unwrap_err(),
            ArtifactError::DigestMismatch { .. }
        ));
    }

    #[test]
    fn a_missing_artifact_says_so_rather_than_failing_the_digest() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = PluginArtifacts::new(dir.path());

        let err = artifacts
            .read_verified(&id(), "0.1.0", &digest(b"anything"))
            .unwrap_err();

        assert!(
            matches!(err, ArtifactError::NotFound(_)),
            "a plugin that was never written is missing, not tampered with; got {err:?}"
        );
    }

    /// The digest is stored as text and compared as text, so a differently
    /// cased record of the same digest must still match.
    #[test]
    fn the_digest_comparison_ignores_hex_case() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = PluginArtifacts::new(dir.path());

        let sha = artifacts.write(&id(), "0.1.0", b"bytes").unwrap();
        artifacts
            .read_verified(&id(), "0.1.0", &sha.to_uppercase())
            .expect("the same digest in a different case is the same digest");
    }

    #[test]
    fn a_version_that_would_escape_the_plugin_directory_is_refused() {
        let artifacts = PluginArtifacts::new("/tmp/does-not-matter");

        for version in ["../../etc/passwd", "1.0.0/../..", "", "a\\b"] {
            assert!(
                artifacts.path_for(&id(), version).is_err(),
                "version {version:?} should not be usable as a filename"
            );
        }
    }

    /// The artifact path puts the version in the filename, not the
    /// directory, so an upgrade adds a file rather than overwriting the only
    /// copy of a build that currently works.
    #[test]
    fn an_upgrade_leaves_the_previous_version_readable() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = PluginArtifacts::new(dir.path());

        let old_sha = artifacts.write(&id(), "0.1.0", b"the old build").unwrap();
        artifacts.write(&id(), "0.2.0", b"the new build").unwrap();

        assert_eq!(
            artifacts.read_verified(&id(), "0.1.0", &old_sha).unwrap(),
            b"the old build",
            "0.1.0 must still be on disk and still verify after 0.2.0 is installed"
        );
    }
}
