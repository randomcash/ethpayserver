//! Root application component with Stripe-inspired layout.

mod header;
mod icons;
mod layout;

use leptos::prelude::*;
use leptos_router::{
    components::{Outlet, ParentRoute, Route, Router, Routes},
    path,
};
use ui_kit::hooks::use_storage::set_local;
use ui_kit::{AuthProvider, LoginPage, RegisterPage};

use crate::api::Store;
use crate::pages::{
    CheckoutPage, DashboardPage, InvoiceDetailPage, InvoicesPage, NotFoundPage, PaymentDetailPage,
    PaymentsPage, SettingsPage, StoreDetailPage, StoresPage, WalletDetailPage, WalletsPage,
};

use layout::ProtectedLayout;

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

                    // Protected routes — single ProtectedLayout, child pages swap via <Outlet/>
                    <ParentRoute path=path!("/") view=ProtectedLayout>
                        <Route path=path!("") view=DashboardPage />
                        <ParentRoute path=path!("/evm") view=|| view! { <Outlet /> }>
                            <Route path=path!("") view=DashboardPage />
                            <Route path=path!("/invoices") view=InvoicesPage />
                            <Route path=path!("/invoices/:id") view=InvoiceDetailPage />
                            <Route path=path!("/payments") view=PaymentsPage />
                            <Route path=path!("/payments/:id") view=PaymentDetailPage />
                            <Route path=path!("/stores") view=StoresPage />
                            <Route path=path!("/stores/:id") view=StoreDetailPage />
                            <Route path=path!("/wallets") view=WalletsPage />
                            <Route path=path!("/wallets/:id") view=WalletDetailPage />
                            <Route path=path!("/settings") view=SettingsPage />
                        </ParentRoute>
                    </ParentRoute>
                </Routes>
            </Router>
        </AuthProvider>
    }
}
