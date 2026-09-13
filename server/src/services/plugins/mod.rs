//! The plugin host: the load-time gate (RCS-264) and the host API surface
//! (RCS-300).
//!
//! `registry` is the manifest-and-version-negotiation slice of
//! [RCS-256](https://linear.app/randomcash/issue/RCS-256): given a parsed
//! [`payserver_plugin_api::Manifest`], decide whether this host will
//! register it.
//!
//! `filter` and `invoice_issuer` are the three capabilities carved out of
//! RCS-256 as [RCS-300](https://linear.app/randomcash/issue/RCS-300): read
//! who the merchants are ([`data_service::MerchantDirectoryReader`], defined
//! in the `data-service` crate rather than here since it is a database read,
//! not a host-state decision), a filter that can refuse invoice creation,
//! and creating an invoice on the instance's own store. Nothing here depends
//! on wasmtime - that dispatch is [RCS-269](https://linear.app/randomcash/issue/RCS-269),
//! and the interface is written and tested against directly, on the
//! assumption that a future wasmtime host-function binding calls into it the
//! same way these tests do.

mod error;
mod filter;
mod invoice_issuer;
mod registry;

pub use error::PluginLoadError;
pub use filter::{
    FilterVerdict, InvoiceCreationFilter, InvoiceCreationFilterRequest,
    run_invoice_creation_filters,
};
pub use invoice_issuer::{
    HostInvoiceIssuer, InvoiceCreateRequest, InvoiceIssuerError, PluginHostApi, enforce_own_store,
};
pub use registry::{PluginRegistry, host_version};
