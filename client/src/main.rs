//! Standalone entry point for the EVM PayServer client.
//!
//! This allows the frontend to run independently for development
//! or as a standalone deployment.

use ethpayserver_client::App;
use leptos::mount::mount_to;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

fn main() {
    console_error_panic_hook::set_once();

    // Log to console that we're starting
    web_sys::console::log_1(&"EVM PayServer: Starting app...".into());

    // Mount to the #app div
    let app_element = document()
        .get_element_by_id("app")
        .expect("Could not find #app element")
        .unchecked_into::<web_sys::HtmlElement>();

    web_sys::console::log_1(&"EVM PayServer: Found #app element, mounting...".into());

    mount_to(app_element, App).forget();

    web_sys::console::log_1(&"EVM PayServer: App mounted successfully".into());
}
