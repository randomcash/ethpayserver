#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Shared fixtures for this module's tests, split by the capability each
//! group dispatches: [`filter`] (invoice-creation filtering), [`observers`]
//! (payment and account-closed notifications) and [`cancel_subscription`]
//! (the one-plugin, one-shot admin action).

use super::*;
use payserver_plugin_api::Manifest;
use payserver_plugin_host::host_version;
use std::time::Duration;
use types::StoreId;
use uuid::Uuid;

mod cancel_subscription;
mod filter;
mod observers;

fn manifest(id: &str, kind: &str, failure_mode: Option<&str>) -> Manifest {
    let mut toml = format!(
        "id = \"{id}\"\nversion = \"0.1.0\"\ndependencies = [\"ethpayserver:^0.1\"]\nkind = \"{kind}\"\nui_schema = 1\n"
    );
    if let Some(mode) = failure_mode {
        toml.push_str(&format!("failure_mode = \"{mode}\"\n"));
    }
    toml.parse().unwrap()
}

fn host() -> Arc<PluginHost> {
    Arc::new(PluginHost::new(
        host_version(),
        3,
        Duration::from_millis(200),
    ))
}

/// A plugin exporting the host ABI (`memory`, `alloc`) plus one hook,
/// whose hook body is supplied. Enough to register, which is what makes
/// `kind` answerable.
fn hook_module(export: &str, body: &str) -> Vec<u8> {
    let text = format!(
        r#"
        (module
            (memory (export "memory") 1)
            (global $next (mut i32) (i32.const 1024))

            (func (export "alloc") (param $len i32) (result i32)
                (local $ptr i32)
                (local.set $ptr (global.get $next))
                (global.set $next (i32.add (global.get $next) (local.get $len)))
                (local.get $ptr))

            (func (export "{export}") (param $ptr i32) (param $len i32) (result i64)
                {body})
        )
        "#
    );
    wat::parse_str(&text).unwrap()
}

/// Registers, and traps on any call - the "could not run" path.
fn trapping(export: &str) -> Vec<u8> {
    hook_module(export, "unreachable")
}

/// Answers with `json`, verbatim, from a data segment. This is the only
/// fixture that exercises the wire contract in the direction a real plugin
/// uses it.
fn answering(export: &str, json: &str) -> Vec<u8> {
    let escaped = json.replace('"', "\\\"");
    let len = json.len();
    let text = format!(
        r#"
        (module
            (memory (export "memory") 1)
            (data (i32.const 0) "{escaped}")
            (global $next (mut i32) (i32.const 1024))

            (func (export "alloc") (param $len i32) (result i32)
                (local $ptr i32)
                (local.set $ptr (global.get $next))
                (global.set $next (i32.add (global.get $next) (local.get $len)))
                (local.get $ptr))

            (func (export "{export}") (param $ptr i32) (param $len i32) (result i64)
                (i64.or
                    (i64.shl (i64.extend_i32_u (i32.const 0)) (i64.const 32))
                    (i64.extend_i32_u (i32.const {len}))))
        )
        "#
    );
    wat::parse_str(&text).unwrap()
}

fn a_store() -> InvoiceCreationFilterRequest {
    InvoiceCreationFilterRequest {
        store_id: StoreId(Uuid::new_v4()),
        account_id: auth::UserId(Uuid::new_v4()),
    }
}
