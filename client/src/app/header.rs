//! Main header bar and WebSocket connection indicator.

use leptos::prelude::*;
use ui_kit::use_auth;
use wasm_bindgen::JsCast;

use crate::api::EvmApiClient;
use crate::components::CreateInvoiceSignal;
use crate::services::ConnectionState;

use super::icons::{IconBell, IconLogout, IconMenu, IconSearch, IconSettings, IconUser};

/// Main header with search and actions.
#[component]
pub(super) fn MainHeader<F>(on_menu_click: F) -> impl IntoView
where
    F: Fn(web_sys::MouseEvent) + 'static,
{
    let auth = use_auth();
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");
    let create_invoice =
        use_context::<CreateInvoiceSignal>().expect("CreateInvoiceSignal must be provided");
    let ws_state =
        use_context::<ReadSignal<ConnectionState>>().expect("ConnectionState must be provided");

    let (menu_open, set_menu_open) = signal(false);
    let menu_ref = leptos::prelude::NodeRef::<leptos::html::Div>::new();

    let toggle_menu = move |_: web_sys::MouseEvent| set_menu_open.update(|v| *v = !*v);

    // Close menu on click outside — single listener, no leak.
    {
        let closure =
            wasm_bindgen::closure::Closure::wrap(Box::new(move |e: web_sys::MouseEvent| {
                if menu_open.get_untracked()
                    && let Some(el) = menu_ref.get_untracked()
                    && let Some(target) =
                        e.target().and_then(|t| t.dyn_into::<web_sys::Node>().ok())
                    && !el.contains(Some(&target))
                {
                    set_menu_open.set(false);
                }
            })
                as Box<dyn FnMut(web_sys::MouseEvent)>);

        let window = web_sys::window().unwrap();
        let _ = window.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
        closure.forget();
    }

    let on_logout = move |_: web_sys::MouseEvent| {
        let api = api.get();
        set_menu_open.set(false);
        leptos::task::spawn_local(async move {
            let _ = api.logout().await;
            auth.logout();
        });
    };

    view! {
        <header class="main-header">
            <div class="main-header-inner">
                <div class="main-header-left">
                    <button class="mobile-menu-toggle" on:click=on_menu_click>
                        <IconMenu />
                    </button>
                    <div class="main-header-search">
                        <IconSearch />
                        <input type="text" placeholder="Search invoices, payments..." />
                    </div>
                </div>

                <div class="main-header-actions">
                    <ConnectionIndicator state=ws_state />
                    <button class="btn btn-ghost btn-sm">
                        <IconBell />
                    </button>
                    <button class="btn btn-primary btn-sm" on:click=move |_| create_invoice.open()>
                        <span>"Create Invoice"</span>
                    </button>
                    <div class="user-menu" node_ref=menu_ref>
                        <button class="btn btn-ghost btn-sm user-menu-trigger" on:click=toggle_menu>
                            <IconUser />
                        </button>
                        <div class=move || if menu_open.get() { "user-menu-dropdown open" } else { "user-menu-dropdown" }>
                            <a href="/evm/settings" class="user-menu-item" on:click=move |_| set_menu_open.set(false)>
                                <IconSettings />
                                <span>"Settings"</span>
                            </a>
                            <div class="user-menu-divider"></div>
                            <button class="user-menu-item user-menu-logout" on:click=on_logout>
                                <IconLogout />
                                <span>"Log out"</span>
                            </button>
                        </div>
                    </div>
                </div>
            </div>
        </header>
    }
}

/// WebSocket connection status indicator.
#[component]
fn ConnectionIndicator(state: ReadSignal<ConnectionState>) -> impl IntoView {
    let class = move || match state.get() {
        ConnectionState::Connected => "ws-indicator ws-indicator-connected",
        ConnectionState::Reconnecting => "ws-indicator ws-indicator-reconnecting",
        ConnectionState::Disconnected => "ws-indicator ws-indicator-disconnected",
    };

    let title = move || match state.get() {
        ConnectionState::Connected => "Live",
        ConnectionState::Reconnecting => "Reconnecting...",
        ConnectionState::Disconnected => "Disconnected",
    };

    let label = move || match state.get() {
        ConnectionState::Connected => "Live",
        ConnectionState::Reconnecting => "Reconnecting",
        ConnectionState::Disconnected => "Offline",
    };

    view! {
        <div class=class title=title>
            <span class="ws-indicator-dot"></span>
            <span class="ws-indicator-label">{label}</span>
        </div>
    }
}
