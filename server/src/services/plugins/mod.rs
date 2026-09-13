//! The plugin host: the load-time gate, the host API surface, and the
//! wasmtime runtime that instantiates, calls, bounds and contains a plugin.
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
//! `runtime` (instantiate/call/deadline/trap on a single plugin) and `host`
//! (action vs. filter dispatch, disable-on-repeated-failure, admin-visible
//! status) are the wasmtime layer that calls into those capabilities.

mod error;
mod filter;
mod host;
mod invoice_issuer;
mod merchant_directory;
mod registry;
mod runtime;

pub use error::PluginLoadError;
pub use filter::{
    FilterVerdict, InvoiceCreationFilter, InvoiceCreationFilterRequest,
    run_invoice_creation_filters,
};
pub use host::{FilterOutcome, PluginHost, PluginHostError, PluginStatusSnapshot};
pub use invoice_issuer::{
    HostInvoiceIssuer, InvoiceCreateRequest, InvoiceIssuerError, PluginHostApi, enforce_own_store,
};
pub use registry::{PluginRegistry, host_version};
pub use runtime::{PluginCallError, PluginEngine, PluginInstance, PluginWasmError};

// Capability 1: no new type here, just `data_service::MerchantDirectoryReader`
// re-exported alongside the other two capabilities' names, and implemented on
// `PluginHostApi` by `merchant_directory` (see that module).
pub use data_service::{MerchantAccount, MerchantDirectoryReader, MerchantStore};
