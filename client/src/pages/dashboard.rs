//! Dashboard page - overview of EVM payment activity.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use ui_kit::components::{Card, CardWithHeader, Grid, PageHeader, StatCard, Stack};

/// Dashboard statistics.
#[derive(Clone, Default, Serialize, Deserialize)]
struct DashboardStats {
    total_invoices: u64,
    pending_invoices: u64,
    total_payments: u64,
    total_volume: String,
}

/// Dashboard page component.
#[component]
pub fn DashboardPage() -> impl IntoView {
    let stats = Resource::new(|| (), |_| async {
        // TODO: Fetch from API
        Ok::<_, String>(DashboardStats {
            total_invoices: 156,
            pending_invoices: 12,
            total_payments: 142,
            total_volume: "$45,230.00".to_string(),
        })
    });

    view! {
        <div class="evm-page">
            <PageHeader
                title="EVM Dashboard"
                description="Overview of your Ethereum payment activity"
            />

            <Suspense fallback=move || view! { <DashboardSkeleton /> }>
                {move || {
                    stats.get().map(|result| {
                        match result {
                            Ok(data) => view! { <DashboardContent stats=data /> }.into_any(),
                            Err(e) => view! {
                                <Card>
                                    <p class="text-error">"Error loading dashboard: " {e}</p>
                                </Card>
                            }.into_any(),
                        }
                    })
                }}
            </Suspense>
        </div>
    }
}

#[component]
fn DashboardContent(stats: DashboardStats) -> impl IntoView {
    view! {
        <Stack gap="lg">
            // Stats grid
            <Grid cols=4 gap="md">
                <StatCard
                    label="Total Invoices"
                    value=stats.total_invoices.to_string()
                />
                <StatCard
                    label="Pending"
                    value=stats.pending_invoices.to_string()
                    change=format!("{} awaiting payment", stats.pending_invoices)
                    positive=false
                />
                <StatCard
                    label="Total Payments"
                    value=stats.total_payments.to_string()
                />
                <StatCard
                    label="Total Volume"
                    value=stats.total_volume.clone()
                    change="+12.5% from last month".to_string()
                    positive=true
                />
            </Grid>

            // Recent activity
            <Grid cols=2 gap="md">
                <CardWithHeader title="Recent Invoices" subtitle="Last 5 invoices">
                    <RecentInvoicesList />
                </CardWithHeader>

                <CardWithHeader title="Recent Payments" subtitle="Last 5 payments">
                    <RecentPaymentsList />
                </CardWithHeader>
            </Grid>

            // Network status
            <CardWithHeader title="Network Status" subtitle="Connected chains">
                <NetworkStatusList />
            </CardWithHeader>
        </Stack>
    }
}

#[component]
fn DashboardSkeleton() -> impl IntoView {
    use ui_kit::components::Skeleton;

    view! {
        <Stack gap="lg">
            <Grid cols=4 gap="md">
                <Skeleton height="100px" />
                <Skeleton height="100px" />
                <Skeleton height="100px" />
                <Skeleton height="100px" />
            </Grid>
            <Grid cols=2 gap="md">
                <Skeleton height="300px" />
                <Skeleton height="300px" />
            </Grid>
        </Stack>
    }
}

#[component]
fn RecentInvoicesList() -> impl IntoView {
    // TODO: Fetch from API
    view! {
        <div class="evm-list">
            <div class="evm-list-empty">
                <p>"No recent invoices"</p>
            </div>
        </div>
    }
}

#[component]
fn RecentPaymentsList() -> impl IntoView {
    // TODO: Fetch from API
    view! {
        <div class="evm-list">
            <div class="evm-list-empty">
                <p>"No recent payments"</p>
            </div>
        </div>
    }
}

#[component]
fn NetworkStatusList() -> impl IntoView {
    use types::Network;
    use ui_kit::components::crypto::NetworkBadge;

    let networks = vec![
        (Network::Ethereum, true),
        (Network::Polygon, true),
        (Network::Arbitrum, true),
        (Network::Optimism, false),
        (Network::Base, true),
    ];

    view! {
        <div class="evm-network-status-list">
            {networks.into_iter().map(|(network, connected)| {
                let status_class = if connected { "connected" } else { "disconnected" };
                view! {
                    <div class=format!("evm-network-status-item {}", status_class)>
                        <NetworkBadge network=network />
                        <span class="evm-network-status-indicator">
                            {if connected { "Connected" } else { "Disconnected" }}
                        </span>
                    </div>
                }
            }).collect_view()}
        </div>
    }
}

/// Dashboard page styles.
pub const DASHBOARD_STYLES: &str = r#"
.evm-list {
    min-height: 200px;
}

.evm-list-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 200px;
    color: var(--ps-text-muted);
}

.evm-network-status-list {
    display: flex;
    flex-wrap: wrap;
    gap: var(--ps-spacing-md);
}

.evm-network-status-item {
    display: flex;
    align-items: center;
    gap: var(--ps-spacing-sm);
    padding: var(--ps-spacing-sm) var(--ps-spacing-md);
    background-color: var(--ps-background);
    border-radius: var(--ps-radius-md);
}

.evm-network-status-indicator {
    font-size: var(--ps-font-sm);
}

.evm-network-status-item.connected .evm-network-status-indicator {
    color: var(--ps-success);
}

.evm-network-status-item.disconnected .evm-network-status-indicator {
    color: var(--ps-error);
}
"#;
