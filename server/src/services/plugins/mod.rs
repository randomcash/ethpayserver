//! The plugin host: the load-time gate, the host API surface, plugin storage,
//! the static page descriptor and its renderer, and the wasmtime runtime that
//! instantiates, calls, bounds and contains a plugin.
//!
//! `registry` is the manifest-and-version-negotiation slice: given a parsed
//! [`payserver_plugin_api::Manifest`], decide whether this host will register
//! it, before any plugin code runs.
//!
//! `merchant_directory`, `filter` and `invoice_issuer` are the three host
//! capabilities: read who the merchants are
//! ([`data_service::MerchantDirectoryReader`], defined in the `data-service`
//! crate rather than here since it is a database read, not a host-state
//! decision, but implemented on [`PluginHostApi`] here so it is reachable the
//! same way the other two capabilities are), a filter that can refuse invoice
//! creation, and creating an invoice on the instance's own store. None of the
//! three depends on wasmtime; each is written and tested against directly.
//!
//! `storage` and `core_data` are the storage slice: a schema per plugin, a
//! migration runner, and typed host calls for a plugin's own schema and for
//! the core data it is allowed to read.
//! `page` and `pages` are the static page descriptor and the renderer host
//! that serves it. No plugin is registered against the renderer until a
//! runtime can produce one, so every page request 404s until then.
//!
//! `runtime` (instantiate/call/deadline/trap on a single plugin) and `host`
//! (action vs. filter dispatch, disable-on-repeated-failure, admin-visible
//! status) are the wasmtime layer that calls into all of the above.

mod core_data;
mod error;
mod filter;
mod host;
mod invoice_issuer;
mod merchant_directory;
pub mod page;
mod pages;
mod registry;
mod runtime;
mod storage;

pub use core_data::{PluginCoreDataApi, PluginStoreSummary};
pub use error::PluginLoadError;
pub use filter::{
    FilterVerdict, InvoiceCreationFilter, InvoiceCreationFilterRequest,
    run_invoice_creation_filters,
};
pub use host::{FilterOutcome, PluginHost, PluginHostError, PluginStatusSnapshot};
pub use invoice_issuer::{
    HostInvoiceIssuer, InvoiceCreateRequest, InvoiceIssuerError, PluginHostApi, enforce_own_store,
};
pub use page::{PageElement, Viewer};
pub use pages::{PageError, PageHost, PageRenderer};
pub use registry::{PluginRegistry, host_version};
pub use runtime::{PluginCallError, PluginEngine, PluginInstance, PluginWasmError};
pub use storage::{PluginSchema, PluginStorage, PluginStorageError};

// Capability 1: no new type here, just `data_service::MerchantDirectoryReader`
// re-exported alongside the other two capabilities' names, and implemented on
// `PluginHostApi` by `merchant_directory` (see that module).
pub use data_service::{MerchantAccount, MerchantDirectoryReader, MerchantStore};
