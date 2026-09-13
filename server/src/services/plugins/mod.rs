//! The plugin host's load-time gate (RCS-264).
//!
//! This is the manifest-and-version-negotiation slice of
//! [RCS-256](https://linear.app/randomcash/issue/RCS-256): given a parsed
//! [`payserver_plugin_api::Manifest`], decide whether this host will
//! register it. No wasmtime, no instantiation, no action/filter dispatch —
//! those need a runtime and stay on RCS-256.

mod error;
mod registry;

pub use error::PluginLoadError;
pub use registry::{PluginRegistry, host_version};
