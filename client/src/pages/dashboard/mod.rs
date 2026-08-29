//! Dashboard page - Stripe-inspired overview of EVM payment activity.
//!
//! Split into the header ([`header`]), the stats cards ([`metrics`]), the
//! charts row ([`charts`]), the recent-activity row ([`activity`]) and the
//! shared inline SVGs ([`icons`]).

mod activity;
mod charts;
mod header;
mod icons;
mod metrics;

use leptos::prelude::*;

use activity::DashboardActivity;
use charts::DashboardCharts;
use header::DashboardHeader;
use metrics::DashboardMetrics;

/// Dashboard page component.
#[component]
pub fn DashboardPage() -> impl IntoView {
    view! {
        <div class="dashboard">
            <DashboardHeader />
            <DashboardMetrics />
            <DashboardCharts />
            <DashboardActivity />
        </div>
    }
}
