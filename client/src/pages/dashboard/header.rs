//! Dashboard header with the page title and top-level actions.

use leptos::prelude::*;
use leptos_router::components::A;

use super::icons::{IconDownload, IconPlus};

/// Dashboard header with title and actions.
#[component]
pub(super) fn DashboardHeader() -> impl IntoView {
    view! {
        <div class="dashboard-header">
            <div>
                <h1 class="dashboard-title">"Dashboard"</h1>
                <p class="dashboard-subtitle">"Overview of your payment activity"</p>
            </div>
            <div class="dashboard-actions">
                <button class="btn btn-secondary btn-sm">
                    <IconDownload />
                    "Export"
                </button>
                <A href="/evm/invoices" attr:class="btn btn-primary btn-sm">
                    <IconPlus />
                    "Create Invoice"
                </A>
            </div>
        </div>
    }
}
