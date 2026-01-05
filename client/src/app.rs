//! Root application component.

use leptos::prelude::*;
use leptos_router::{
    components::{Route, Router, Routes},
    path,
};

use crate::pages::{
    DashboardPage, InvoiceDetailPage, InvoicesPage, NotFoundPage, PaymentsPage, SettingsPage,
    StoreDetailPage, StoresPage, WalletsPage,
};

/// Root application component.
#[component]
pub fn App() -> impl IntoView {
    view! {
        <div style="min-height: 100vh; background: white;">
            <Router>
                <header style="padding: 1rem; background: #2563eb; color: white;">
                    <h1 style="margin: 0; font-size: 1.25rem;">"EVM PayServer"</h1>
                </header>
                <nav style="padding: 0.75rem 1rem; background: #f1f5f9; border-bottom: 1px solid #e2e8f0;">
                    <a href="/" style="margin-right: 1rem; color: #2563eb;">"Dashboard"</a>
                    <a href="/evm/invoices" style="margin-right: 1rem; color: #2563eb;">"Invoices"</a>
                    <a href="/evm/payments" style="margin-right: 1rem; color: #2563eb;">"Payments"</a>
                    <a href="/evm/stores" style="margin-right: 1rem; color: #2563eb;">"Stores"</a>
                    <a href="/evm/wallets" style="margin-right: 1rem; color: #2563eb;">"Wallets"</a>
                    <a href="/evm/settings" style="color: #2563eb;">"Settings"</a>
                </nav>
                <main style="padding: 1.5rem;">
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
            </Router>
        </div>
    }
}

/// Application styles.
pub const APP_STYLES: &str = r#"
.evm-client {
    min-height: 100vh;
    background-color: var(--ps-background);
    color: var(--ps-text);
}

.evm-page {
    padding: var(--ps-spacing-lg);
    max-width: 1200px;
    margin: 0 auto;
}

.evm-page-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: var(--ps-spacing-xl);
}

.evm-page-title {
    font-size: 1.5rem;
    font-weight: 600;
    margin: 0;
}

.evm-page-actions {
    display: flex;
    gap: var(--ps-spacing-sm);
}
"#;
