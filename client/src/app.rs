//! Root application component with Stripe-inspired layout.

use leptos::prelude::*;
use leptos::children::Children;
use leptos_router::{
    components::{Route, Router, Routes, A},
    hooks::use_location,
    path,
};

use crate::pages::{
    DashboardPage, InvoiceDetailPage, InvoicesPage, NotFoundPage, PaymentsPage, SettingsPage,
    StoreDetailPage, StoresPage, WalletsPage,
};

/// Root application component.
#[component]
pub fn App() -> impl IntoView {
    // Mobile sidebar state
    let (sidebar_open, set_sidebar_open) = signal(false);

    let close_sidebar = move |_| set_sidebar_open.set(false);
    let toggle_sidebar = move |_| set_sidebar_open.update(|v| *v = !*v);

    view! {
        <Router>
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
                        <Routes fallback=|| view! { <NotFoundPage /> }>
                            <Route path=path!("/") view=DashboardPage />
                            <Route path=path!("/evm") view=DashboardPage />
                            <Route path=path!("/evm/invoices") view=InvoicesPage />
                            <Route path=path!("/evm/invoices/:id") view=InvoiceDetailPage />
                            <Route path=path!("/evm/payments") view=PaymentsPage />
                            <Route path=path!("/evm/stores") view=StoresPage />
                            <Route path=path!("/evm/stores/:id") view=StoreDetailPage />
                            <Route path=path!("/evm/wallets") view=WalletsPage />
                            <Route path=path!("/evm/settings") view=SettingsPage />
                        </Routes>
                    </main>
                </div>
            </div>
        </Router>
    }
}

/// Sidebar navigation component.
#[component]
fn Sidebar<F>(
    open: ReadSignal<bool>,
    on_close: F,
) -> impl IntoView
where
    F: Fn(web_sys::MouseEvent) + 'static + Clone,
{
    let on_close_link = on_close.clone();

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

            <nav class="sidebar-nav" on:click=move |e| on_close_link(e)>
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
                <div class="sidebar-link">
                    <IconHelp />
                    <span>"Documentation"</span>
                </div>
            </div>
        </aside>
    }
}

/// Sidebar navigation link.
#[component]
fn SidebarLink(
    href: &'static str,
    label: &'static str,
    children: Children,
) -> impl IntoView {
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
                    <button class="btn btn-ghost btn-sm">
                        <IconBell />
                    </button>
                    <button class="btn btn-primary btn-sm">
                        <span>"Create Invoice"</span>
                    </button>
                </div>
            </div>
        </header>
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
