//! Root application component with Stripe-inspired layout.

use std::rc::Rc;

use leptos::children::{Children, ChildrenFn};
use leptos::prelude::*;
use leptos_router::{
    components::{A, Route, Router, Routes},
    hooks::use_location,
    path,
};
use ui_kit::hooks::use_storage::{get_local, set_local};
use ui_kit::{AuthGuard, AuthProvider, LoginPage, RegisterPage, use_auth};
use wasm_bindgen::JsCast;

use crate::api::{EvmApiClient, Store};
use crate::components::{CreateInvoiceModal, CreateInvoiceSignal};
use crate::pages::{
    CheckoutPage, DashboardPage, InvoiceDetailPage, InvoicesPage, NotFoundPage, PaymentDetailPage,
    PaymentsPage, SettingsPage, StoreDetailPage, StoresPage, WalletDetailPage, WalletsPage,
};
use crate::services::{ConnectionState, WebSocketService};

/// localStorage key for persisted selected store ID.
const SELECTED_STORE_KEY: &str = "eps_selected_store";

/// Store context provided to all authenticated pages.
///
/// Tracks available stores and the currently selected store,
/// persisting the selection in localStorage.
#[derive(Clone)]
pub struct StoreContext {
    /// All stores the user has access to.
    pub stores: ReadSignal<Vec<Store>>,
    /// Currently selected store (None = "All Stores").
    pub selected_store_id: ReadSignal<Option<String>>,
    /// Set the selected store (pass None for "All Stores").
    set_selected: WriteSignal<Option<String>>,
    /// Trigger a refetch of stores.
    refetch: ArcTrigger,
}

impl StoreContext {
    /// Set the currently selected store. Persists to localStorage.
    pub fn select_store(&self, store_id: Option<String>) {
        if let Some(ref id) = store_id {
            let _ = set_local(SELECTED_STORE_KEY, id);
        } else {
            ui_kit::hooks::use_storage::remove_local(SELECTED_STORE_KEY);
        }
        self.set_selected.set(store_id);
    }

    /// Get the currently selected store object.
    pub fn selected_store(&self) -> Option<Store> {
        let id = self.selected_store_id.get();
        let stores = self.stores.get();
        id.and_then(|id| stores.iter().find(|s| s.id == id).cloned())
    }

    /// Trigger a refetch of the stores list.
    pub fn refetch_stores(&self) {
        self.refetch.notify();
    }
}

/// Root application component.
#[component]
pub fn App() -> impl IntoView {
    view! {
        <AuthProvider>
            <Router>
                <Routes fallback=|| view! { <NotFoundPage /> }>
                    // Public routes (no layout)
                    <Route path=path!("/login") view=|| view! { <LoginPage /> } />
                    <Route path=path!("/register") view=|| view! { <RegisterPage /> } />
                    <Route path=path!("/checkout/:id") view=|| view! { <CheckoutPage /> } />

                    // Protected routes with layout
                    <Route path=path!("/") view=DashboardProtected />
                    <Route path=path!("/evm") view=DashboardProtected />
                    <Route path=path!("/evm/invoices") view=InvoicesProtected />
                    <Route path=path!("/evm/invoices/:id") view=InvoiceDetailProtected />
                    <Route path=path!("/evm/payments") view=PaymentsProtected />
                    <Route path=path!("/evm/payments/:id") view=PaymentDetailProtected />
                    <Route path=path!("/evm/stores") view=StoresProtected />
                    <Route path=path!("/evm/stores/:id") view=StoreDetailProtected />
                    <Route path=path!("/evm/wallets") view=WalletsProtected />
                    <Route path=path!("/evm/wallets/:id") view=WalletDetailProtected />
                    <Route path=path!("/evm/settings") view=SettingsProtected />
                </Routes>
            </Router>
        </AuthProvider>
    }
}

// Protected route wrappers
#[component]
fn DashboardProtected() -> impl IntoView {
    view! { <ProtectedLayout><DashboardPage /></ProtectedLayout> }
}

#[component]
fn InvoicesProtected() -> impl IntoView {
    view! { <ProtectedLayout><InvoicesPage /></ProtectedLayout> }
}

#[component]
fn InvoiceDetailProtected() -> impl IntoView {
    view! { <ProtectedLayout><InvoiceDetailPage /></ProtectedLayout> }
}

#[component]
fn PaymentsProtected() -> impl IntoView {
    view! { <ProtectedLayout><PaymentsPage /></ProtectedLayout> }
}

#[component]
fn PaymentDetailProtected() -> impl IntoView {
    view! { <ProtectedLayout><PaymentDetailPage /></ProtectedLayout> }
}

#[component]
fn StoresProtected() -> impl IntoView {
    view! { <ProtectedLayout><StoresPage /></ProtectedLayout> }
}

#[component]
fn StoreDetailProtected() -> impl IntoView {
    view! { <ProtectedLayout><StoreDetailPage /></ProtectedLayout> }
}

#[component]
fn WalletsProtected() -> impl IntoView {
    view! { <ProtectedLayout><WalletsPage /></ProtectedLayout> }
}

#[component]
fn WalletDetailProtected() -> impl IntoView {
    view! { <ProtectedLayout><WalletDetailPage /></ProtectedLayout> }
}

#[component]
fn SettingsProtected() -> impl IntoView {
    view! { <ProtectedLayout><SettingsPage /></ProtectedLayout> }
}

/// Protected layout with AuthGuard and app shell.
#[component]
fn ProtectedLayout(children: ChildrenFn) -> impl IntoView {
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
                    // If the server rejected our session, log out so we redirect to login
                    // instead of showing a stalled loading state.
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
                    let url = format!("{}://{}/ws?token={}", protocol, host, t);
                    if let Err(e) = ws.connect(&url) {
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
                        {children()}
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

/// Main header with search and actions.
#[component]
fn MainHeader<F>(on_menu_click: F) -> impl IntoView
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

// ============================================
// SVG Icons (inline for simplicity)
// ============================================

#[component]
fn IconDashboard() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="3" width="7" height="7"></rect>
            <rect x="14" y="3" width="7" height="7"></rect>
            <rect x="14" y="14" width="7" height="7"></rect>
            <rect x="3" y="14" width="7" height="7"></rect>
        </svg>
    }
}

#[component]
fn IconInvoice() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"></path>
            <polyline points="14 2 14 8 20 8"></polyline>
            <line x1="16" y1="13" x2="8" y2="13"></line>
            <line x1="16" y1="17" x2="8" y2="17"></line>
            <polyline points="10 9 9 9 8 9"></polyline>
        </svg>
    }
}

#[component]
fn IconPayment() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="1" y="4" width="22" height="16" rx="2" ry="2"></rect>
            <line x1="1" y1="10" x2="23" y2="10"></line>
        </svg>
    }
}

#[component]
fn IconStore() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"></path>
            <polyline points="9 22 9 12 15 12 15 22"></polyline>
        </svg>
    }
}

#[component]
fn IconWallet() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 12V7H5a2 2 0 0 1 0-4h14v4"></path>
            <path d="M3 5v14a2 2 0 0 0 2 2h16v-5"></path>
            <path d="M18 12a2 2 0 0 0 0 4h4v-4Z"></path>
        </svg>
    }
}

#[component]
fn IconSettings() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="3"></circle>
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"></path>
        </svg>
    }
}

#[component]
fn IconHelp() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10"></circle>
            <path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3"></path>
            <line x1="12" y1="17" x2="12.01" y2="17"></line>
        </svg>
    }
}

#[component]
fn IconSearch() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="color: var(--text-muted);">
            <circle cx="11" cy="11" r="8"></circle>
            <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
        </svg>
    }
}

#[component]
fn IconBell() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9"></path>
            <path d="M13.73 21a2 2 0 0 1-3.46 0"></path>
        </svg>
    }
}

#[component]
fn IconMenu() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="3" y1="12" x2="21" y2="12"></line>
            <line x1="3" y1="6" x2="21" y2="6"></line>
            <line x1="3" y1="18" x2="21" y2="18"></line>
        </svg>
    }
}

#[component]
fn IconClose() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="18" y1="6" x2="6" y2="18"></line>
            <line x1="6" y1="6" x2="18" y2="18"></line>
        </svg>
    }
}

#[component]
fn IconChevron(expanded: ReadSignal<bool>) -> impl IntoView {
    view! {
        <svg
            xmlns="http://www.w3.org/2000/svg"
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            class="icon-chevron"
            style=move || if expanded.get() { "transform: rotate(180deg)" } else { "" }
        >
            <polyline points="6 9 12 15 18 9"></polyline>
        </svg>
    }
}

#[component]
fn IconLayers() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polygon points="12 2 2 7 12 12 22 7 12 2"></polygon>
            <polyline points="2 17 12 22 22 17"></polyline>
            <polyline points="2 12 12 17 22 12"></polyline>
        </svg>
    }
}

#[component]
fn IconCheck() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="20 6 9 17 4 12"></polyline>
        </svg>
    }
}

#[component]
fn IconUser() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"></path>
            <circle cx="12" cy="7" r="4"></circle>
        </svg>
    }
}

#[component]
fn IconLogout() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"></path>
            <polyline points="16 17 21 12 16 7"></polyline>
            <line x1="21" y1="12" x2="9" y2="12"></line>
        </svg>
    }
}
