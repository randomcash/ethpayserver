//! Protected app shell: layout, sidebar navigation, and shared wiring.

use std::rc::Rc;

use leptos::children::Children;
use leptos::prelude::*;
use leptos_router::{
    components::{A, Outlet},
    hooks::use_location,
};
use ui_kit::hooks::use_storage::{get_local, set_local};
use ui_kit::{AuthGuard, use_auth};

use crate::api::{EvmApiClient, Store};
use crate::components::{CreateInvoiceModal, CreateInvoiceSignal};
use crate::services::WebSocketService;

use super::header::MainHeader;
use super::icons::{
    IconCheck, IconChevron, IconClose, IconDashboard, IconHelp, IconInvoice, IconLayers,
    IconPayment, IconSettings, IconStore, IconWallet,
};
use super::{SELECTED_STORE_KEY, StoreContext};

/// Protected layout with AuthGuard and app shell.
///
/// Mounted once via `<ParentRoute>` — child pages render in the `<Outlet/>`.
#[component]
pub(super) fn ProtectedLayout() -> impl IntoView {
    // Mobile sidebar state
    let (sidebar_open, set_sidebar_open) = signal(false);

    let close_sidebar = move |_| set_sidebar_open.set(false);
    let toggle_sidebar = move |_| set_sidebar_open.update(|v| *v = !*v);

    // Provide authenticated API client to all child pages.
    // Reactively updates when the auth token changes.
    let auth = use_auth();
    let api = Signal::derive(move || {
        let token = auth.token.get();
        EvmApiClient::new("").with_token(token)
    });
    provide_context(api);

    // Store context: fetch stores, track selection, persist in localStorage.
    let (stores, set_stores) = signal(Vec::<Store>::new());
    let saved_id: Option<String> = get_local(SELECTED_STORE_KEY);
    let (selected_store_id, set_selected_store_id) = signal(saved_id);
    let refetch = ArcTrigger::new();

    // Fetch stores on mount and whenever refetch is triggered.
    let refetch_for_effect = refetch.clone();
    Effect::new(move || {
        refetch_for_effect.track();
        // Don't fetch protected data until authenticated. Reading `api` here would
        // track `auth.token`, and `logout()` on a 401 writes that token — a reactive
        // feedback loop that races the login redirect. Gating on auth state breaks it.
        if !matches!(auth.state.get(), ui_kit::types::AuthState::Authenticated(_)) {
            return;
        }
        let api = api.get();
        leptos::task::spawn_local(async move {
            match api.list_stores().await {
                Ok(fetched) => {
                    // If selected store no longer exists, clear selection.
                    if let Some(ref id) = selected_store_id.get_untracked()
                        && !fetched.iter().any(|s| &s.id == id)
                    {
                        set_selected_store_id.set(None);
                        ui_kit::hooks::use_storage::remove_local(SELECTED_STORE_KEY);
                    }
                    // Auto-select the first store if none is selected.
                    if selected_store_id.get_untracked().is_none()
                        && let Some(first) = fetched.first()
                    {
                        let id = first.id.clone();
                        let _ = set_local(SELECTED_STORE_KEY, &id);
                        set_selected_store_id.set(Some(id));
                    }
                    set_stores.set(fetched);
                }
                Err(e) => {
                    web_sys::console::error_1(&format!("Failed to fetch stores: {}", e).into());
                    let msg = e.to_string();
                    if msg.contains("Unauthorized") || msg.contains("401") {
                        auth.logout();
                    }
                }
            }
        });
    });

    let store_ctx = StoreContext {
        stores,
        selected_store_id,
        set_selected: set_selected_store_id,
        refetch,
    };
    provide_context(store_ctx);

    // WebSocket service — single connection shared by all child pages.
    let ws = WebSocketService::new();
    let ws_connection_state = ws.connection_state();
    let ws_last_update = ws.last_update();

    // Connect WebSocket when auth token is available, disconnect on logout.
    let ws_for_effect = Rc::new(ws);
    provide_context(ws_connection_state);
    provide_context(ws_last_update);

    {
        let ws = ws_for_effect.clone();
        Effect::new(move || {
            let token = auth.token.get();
            match token {
                Some(ref t) => {
                    let protocol = if web_sys::window()
                        .and_then(|w| w.location().protocol().ok())
                        .as_deref()
                        == Some("https:")
                    {
                        "wss"
                    } else {
                        "ws"
                    };
                    let host = web_sys::window()
                        .and_then(|w| w.location().host().ok())
                        .unwrap_or_default();
                    let url = format!("{}://{}/ws", protocol, host);
                    if let Err(e) = ws.connect(&url, Some(t)) {
                        web_sys::console::error_1(
                            &format!("WebSocket connect error: {}", e).into(),
                        );
                    }
                }
                None => {
                    ws.disconnect();
                }
            }
        });
    }

    // Disconnect on drop (component unmount).
    let ws_for_cleanup = send_wrapper::SendWrapper::new(ws_for_effect.clone());
    on_cleanup(move || {
        ws_for_cleanup.disconnect();
    });

    // Create invoice modal signal — shared between header button and invoices page.
    let (show_create_invoice, set_show_create_invoice) = signal(false);
    let create_invoice_signal = CreateInvoiceSignal {
        show: show_create_invoice,
        set_show: set_show_create_invoice,
    };
    provide_context(create_invoice_signal);

    view! {
        <AuthGuard>
            <div class="app-layout">
                // Mobile overlay
                <div
                    class=move || if sidebar_open.get() { "sidebar-overlay open" } else { "sidebar-overlay" }
                    on:click=close_sidebar
                ></div>

                <Sidebar open=sidebar_open on_close=close_sidebar />
                <div class="main-content">
                    <MainHeader on_menu_click=toggle_sidebar />
                    <main class="page-container">
                        <Outlet />
                    </main>
                </div>
            </div>
            <CreateInvoiceModal />
        </AuthGuard>
    }
}

/// Sidebar navigation component.
#[component]
fn Sidebar<F>(open: ReadSignal<bool>, on_close: F) -> impl IntoView
where
    F: Fn(web_sys::MouseEvent) + 'static + Clone,
{
    let on_close_link = on_close.clone();

    // Store selector state
    let (dropdown_open, set_dropdown_open) = signal(false);

    // Get store context
    let store_ctx = use_context::<StoreContext>().expect("StoreContext must be provided");

    let toggle_dropdown = move |_| set_dropdown_open.update(|v| *v = !*v);

    // Selected store name for display
    let selected_name = {
        let store_ctx = store_ctx.clone();
        move || match store_ctx.selected_store() {
            Some(s) => s.name.clone(),
            None => "All Stores".to_string(),
        }
    };

    view! {
        <aside class=move || if open.get() { "sidebar open" } else { "sidebar" }>
            <div class="sidebar-header">
                <div class="sidebar-logo">
                    <span class="sidebar-logo-icon">"E"</span>
                    <span>"ETHPayServer"</span>
                </div>
                <button class="sidebar-close" on:click=on_close>
                    <IconClose />
                </button>
            </div>

            // Store Selector
            <div class="store-selector">
                <button
                    class="store-selector-btn"
                    on:click=toggle_dropdown
                >
                    <div class="store-selector-info">
                        <IconStore />
                        <span class="store-selector-name">{selected_name.clone()}</span>
                    </div>
                    <IconChevron expanded=dropdown_open />
                </button>

                <div class=move || if dropdown_open.get() { "store-dropdown open" } else { "store-dropdown" }>
                    // "All Stores" option
                    {
                        let store_ctx = store_ctx.clone();
                        view! {
                            <button
                                class=move || if store_ctx.selected_store_id.get().is_none() { "store-dropdown-item active" } else { "store-dropdown-item" }
                                on:click={
                                    let store_ctx = store_ctx.clone();
                                    move |_| {
                                        store_ctx.select_store(None);
                                        set_dropdown_open.set(false);
                                    }
                                }
                            >
                                <IconLayers />
                                <span>"All Stores"</span>
                                {
                                    let store_ctx = store_ctx.clone();
                                    move || store_ctx.selected_store_id.get().is_none().then(|| view! { <IconCheck /> })
                                }
                            </button>
                        }
                    }

                    // Store list from API
                    <For
                        each=move || store_ctx.stores.get()
                        key=|store| store.id.clone()
                        let:store
                    >
                        {
                            let store_id = store.id.clone();
                            let store_name = store.name.clone();
                            let store_ctx = store_ctx.clone();
                            let id_for_click = store_id.clone();
                            let id_for_active = store_id.clone();
                            view! {
                                <button
                                    class=move || {
                                        let is_active = store_ctx.selected_store_id.get().as_deref() == Some(&id_for_active);
                                        if is_active { "store-dropdown-item active" } else { "store-dropdown-item" }
                                    }
                                    on:click={
                                        let store_ctx = store_ctx.clone();
                                        let id = id_for_click.clone();
                                        move |_| {
                                            store_ctx.select_store(Some(id.clone()));
                                            set_dropdown_open.set(false);
                                        }
                                    }
                                >
                                    <IconStore />
                                    <span>{store_name.clone()}</span>
                                    {
                                        let store_ctx = store_ctx.clone();
                                        let id = store_id.clone();
                                        move || {
                                            let is_active = store_ctx.selected_store_id.get().as_deref() == Some(&id);
                                            is_active.then(|| view! { <IconCheck /> })
                                        }
                                    }
                                </button>
                            }
                        }
                    </For>

                    <div class="store-dropdown-divider"></div>
                    <a href="/evm/stores" class="store-dropdown-item store-dropdown-manage"
                        on:click=move |_| set_dropdown_open.set(false)
                    >
                        <IconSettings />
                        <span>"Manage Stores"</span>
                    </a>
                </div>
            </div>

            <nav class="sidebar-nav" on:click=on_close_link>
                // Main Navigation
                <div class="sidebar-section">
                    <SidebarLink href="/" label="Dashboard">
                        <IconDashboard />
                    </SidebarLink>
                    <SidebarLink href="/evm/invoices" label="Invoices">
                        <IconInvoice />
                    </SidebarLink>
                    <SidebarLink href="/evm/payments" label="Payments">
                        <IconPayment />
                    </SidebarLink>
                </div>

                // Configuration Section
                <div class="sidebar-section">
                    <div class="sidebar-section-title">"Configuration"</div>
                    <SidebarLink href="/evm/stores" label="Stores">
                        <IconStore />
                    </SidebarLink>
                    <SidebarLink href="/evm/wallets" label="Wallets">
                        <IconWallet />
                    </SidebarLink>
                    <SidebarLink href="/evm/settings" label="Settings">
                        <IconSettings />
                    </SidebarLink>
                </div>
            </nav>

            <div class="sidebar-footer">
                <a href="https://docs.random.cash" target="_blank" class="sidebar-link">
                    <IconHelp />
                    <span>"Documentation"</span>
                </a>
            </div>
        </aside>
    }
}

/// Sidebar navigation link.
#[component]
fn SidebarLink(href: &'static str, label: &'static str, children: Children) -> impl IntoView {
    let location = use_location();
    let is_active = move || {
        let path = location.pathname.get();
        if href == "/" {
            path == "/" || path == "/evm"
        } else {
            path.starts_with(href)
        }
    };

    view! {
        <A
            href=href
            attr:class=move || if is_active() { "sidebar-link active" } else { "sidebar-link" }
        >
            <span class="sidebar-link-icon">
                {children()}
            </span>
            <span>{label}</span>
        </A>
    }
}
