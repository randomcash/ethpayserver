//! EVM PayServer Web Client
//!
//! Leptos-based frontend for ethpayserver, providing:
//! - Invoice management
//! - Payment monitoring
//! - Store configuration
//! - Wallet management
//!
//! Can run standalone or be loaded as a module in the dashboard aggregator.

#![allow(clippy::items_after_test_module)]

pub mod api;
pub mod app;
pub mod components;
pub mod pages;

pub use app::App;

use types::Network;
use ui_kit::module::{CheckoutPluginConfig, ModuleInfo};
use ui_kit::types::RouteInfo;
use wasm_bindgen::prelude::*;

/// EVM module identifier.
pub const MODULE_ID: &str = "evm";

/// EVM module display name.
pub const MODULE_NAME: &str = "Ethereum";

/// Get module info for the dashboard aggregator.
pub fn module_info(api_url: String) -> ModuleInfo {
    ModuleInfo {
        id: MODULE_ID,
        name: MODULE_NAME,
        icon: "ethereum",
        api_url,
        routes: vec![
            RouteInfo {
                path: "/evm".to_string(),
                label: "Dashboard".to_string(),
                icon: Some("dashboard".to_string()),
            },
            RouteInfo {
                path: "/evm/invoices".to_string(),
                label: "Invoices".to_string(),
                icon: Some("receipt".to_string()),
            },
            RouteInfo {
                path: "/evm/payments".to_string(),
                label: "Payments".to_string(),
                icon: Some("payments".to_string()),
            },
            RouteInfo {
                path: "/evm/stores".to_string(),
                label: "Stores".to_string(),
                icon: Some("store".to_string()),
            },
            RouteInfo {
                path: "/evm/wallets".to_string(),
                label: "Wallets".to_string(),
                icon: Some("wallet".to_string()),
            },
            RouteInfo {
                path: "/evm/settings".to_string(),
                label: "Settings".to_string(),
                icon: Some("settings".to_string()),
            },
        ],
    }
}

/// Get checkout plugin configuration for the checkout page.
pub fn checkout_plugin() -> CheckoutPluginConfig {
    CheckoutPluginConfig {
        module_id: MODULE_ID,
        network_badge: render_network_badge,
        amount_details: Some(render_amount_details),
        qr_code: render_qr_code,
        wallet_actions: Some(render_wallet_actions),
    }
}

// Checkout slot implementations
fn render_network_badge(_chain_id: u64, network_name: &str) -> leptos::prelude::AnyView {
    use leptos::prelude::*;
    use ui_kit::components::crypto::NetworkBadge;

    // Try to parse network from name
    let network = network_name.parse::<Network>().unwrap_or(Network::Ethereum);

    view! {
        <NetworkBadge network=network />
    }
    .into_any()
}

fn render_amount_details(chain_id: u64, _amount: &str) -> Option<leptos::prelude::AnyView> {
    use leptos::prelude::*;

    // Show gas info for EVM chains
    Some(
        view! {
            <div class="evm-amount-details">
                <span class="evm-chain-info">"Chain ID: " {chain_id}</span>
            </div>
        }
        .into_any(),
    )
}

fn render_qr_code(payment_request: &str) -> leptos::prelude::AnyView {
    use leptos::prelude::*;
    use ui_kit::components::crypto::QrCodeCard;

    let data = payment_request.to_string();

    view! {
        <QrCodeCard data=data label="Scan to pay" size=250 />
    }
    .into_any()
}

fn render_wallet_actions(
    _payment_address: &str,
    _chain_id: u64,
) -> Option<leptos::prelude::AnyView> {
    use leptos::prelude::*;

    // WalletConnect integration would go here
    Some(
        view! {
            <div class="evm-wallet-actions">
                <button class="ps-btn ps-btn-primary">
                    "Connect Wallet"
                </button>
            </div>
        }
        .into_any(),
    )
}

/// Initialize and mount the app (called when loaded as WASM module).
#[wasm_bindgen(start)]
pub fn init() {
    use leptos::mount::mount_to;
    use leptos::prelude::*;
    use wasm_bindgen::JsCast;

    console_error_panic_hook::set_once();
    web_sys::console::log_1(&"EVM PayServer: Starting...".into());

    // Mount to the #app div
    let app_element = document()
        .get_element_by_id("app")
        .expect("Could not find #app element")
        .unchecked_into::<web_sys::HtmlElement>();

    web_sys::console::log_1(&"EVM PayServer: Mounting app...".into());

    mount_to(app_element, App).forget();

    web_sys::console::log_1(&"EVM PayServer: App mounted!".into());
}

/// Public init function that can be called manually.
pub fn initialize() {
    console_error_panic_hook::set_once();
}
