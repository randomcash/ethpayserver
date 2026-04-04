//! Dashboard page - Stripe-inspired overview of EVM payment activity.

use leptos::prelude::*;
use leptos_router::components::A;
use crate::api::{DashboardStats, EvmApiClient};

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

/// Dashboard header with title and actions.
#[component]
fn DashboardHeader() -> impl IntoView {
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

/// Key metrics section — fetches real data from the dashboard stats API.
#[component]
fn DashboardMetrics() -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");

    let stats_resource = LocalResource::new(move || {
        let client = api.get();
        async move { client.get_dashboard_stats().await.ok() }
    });

    view! {
        <Suspense fallback=move || view! {
            <div class="metrics-grid">
                <MetricCard label="Total invoices" value="--" change="" trend="neutral" period="loading" />
                <MetricCard label="Paid invoices" value="--" change="" trend="neutral" period="loading" />
                <MetricCard label="Pending invoices" value="--" change="" trend="neutral" period="loading" />
                <MetricCard label="Total payments" value="--" change="" trend="neutral" period="loading" />
            </div>
        }>
            {move || Suspend::new(async move {
                match stats_resource.await {
                    Some(stats) => {
                        let total_inv = stats.total_invoices.to_string();
                        let paid = stats.paid_invoices.to_string();
                        let pending = stats.pending_invoices.to_string();
                        let payments = stats.total_payments.to_string();
                        let stores_label = format!("{} stores", stats.total_stores);

                        view! {
                            <div class="metrics-grid">
                                <MetricCard
                                    label="Total invoices"
                                    value=total_inv
                                    change=""
                                    trend="neutral"
                                    period=stores_label.clone()
                                />
                                <MetricCard
                                    label="Paid invoices"
                                    value=paid
                                    change=""
                                    trend="up"
                                    period="completed"
                                />
                                <MetricCard
                                    label="Pending invoices"
                                    value=pending
                                    change=""
                                    trend="neutral"
                                    period="awaiting payment"
                                />
                                <MetricCard
                                    label="Total payments"
                                    value=payments
                                    change=""
                                    trend="up"
                                    period="received"
                                />
                            </div>
                        }.into_any()
                    }
                    None => view! {
                        <div class="metrics-grid">
                            <MetricCard label="Total invoices" value="--" change="" trend="neutral" period="unavailable" />
                            <MetricCard label="Paid invoices" value="--" change="" trend="neutral" period="unavailable" />
                            <MetricCard label="Pending invoices" value="--" change="" trend="neutral" period="unavailable" />
                            <MetricCard label="Total payments" value="--" change="" trend="neutral" period="unavailable" />
                        </div>
                    }.into_any(),
                }
            })}
        </Suspense>
    }
}

/// Individual metric card.
#[component]
fn MetricCard(
    label: &'static str,
    #[prop(into)] value: String,
    change: &'static str,
    trend: &'static str,
    #[prop(into)] period: String,
) -> impl IntoView {
    let trend_class = match trend {
        "up" => "metric-trend metric-trend-up",
        "down" => "metric-trend metric-trend-down",
        _ => "metric-trend metric-trend-neutral",
    };

    view! {
        <div class="metric-card">
            <div class="metric-label">{label}</div>
            <div class="metric-value">{value}</div>
            <div class="metric-footer">
                <span class=trend_class>
                    {match trend {
                        "up" => view! { <IconTrendUp /> }.into_any(),
                        "down" => view! { <IconTrendDown /> }.into_any(),
                        _ => view! { <IconMinus /> }.into_any(),
                    }}
                    {change}
                </span>
                <span class="metric-period">{period}</span>
            </div>
        </div>
    }
}

/// Charts section.
#[component]
fn DashboardCharts() -> impl IntoView {
    view! {
        <div class="charts-section">
            <div class="chart-card chart-card-main">
                <div class="chart-header">
                    <div>
                        <h3 class="chart-title">"Payment volume"</h3>
                        <p class="chart-subtitle">"Daily payment volume over the last 30 days"</p>
                    </div>
                    <div class="chart-controls">
                        <button class="btn btn-ghost btn-xs active">"7D"</button>
                        <button class="btn btn-ghost btn-xs">"30D"</button>
                        <button class="btn btn-ghost btn-xs">"90D"</button>
                    </div>
                </div>
                <div class="chart-body">
                    <VolumeChart />
                </div>
            </div>

            <div class="chart-card">
                <div class="chart-header">
                    <h3 class="chart-title">"Payment methods"</h3>
                </div>
                <div class="chart-body">
                    <PaymentMethodsBreakdown />
                </div>
            </div>
        </div>
    }
}

/// Simple volume chart visualization.
#[component]
fn VolumeChart() -> impl IntoView {
    // Mock data for chart bars
    let data = vec![
        35, 42, 28, 55, 48, 62, 45, 72, 58, 65, 78, 52, 88, 75, 92, 68, 85, 72, 95, 82, 78, 88, 92,
        85, 98, 75, 82, 90, 95, 100,
    ];
    let max = 100.0_f64;

    view! {
        <div class="volume-chart">
            <div class="volume-chart-bars">
                {data.into_iter().enumerate().map(|(i, val)| {
                    let height = (val as f64 / max * 100.0) as u32;
                    let is_today = i == 29;
                    view! {
                        <div
                            class=if is_today { "volume-bar volume-bar-today" } else { "volume-bar" }
                            style=format!("height: {}%", height)
                        ></div>
                    }
                }).collect_view()}
            </div>
            <div class="volume-chart-labels">
                <span>"30 days ago"</span>
                <span>"Today"</span>
            </div>
        </div>
    }
}

/// Payment methods breakdown.
#[component]
fn PaymentMethodsBreakdown() -> impl IntoView {
    let methods = vec![
        ("ETH", 45, "#627eea"),
        ("USDC", 32, "#2775ca"),
        ("USDT", 18, "#26a17b"),
        ("DAI", 5, "#f5ac37"),
    ];

    view! {
        <div class="payment-methods">
            {methods.into_iter().map(|(name, pct, color)| {
                view! {
                    <div class="payment-method-row">
                        <div class="payment-method-info">
                            <span class="payment-method-dot" style=format!("background: {}", color)></span>
                            <span class="payment-method-name">{name}</span>
                        </div>
                        <div class="payment-method-bar-container">
                            <div
                                class="payment-method-bar"
                                style=format!("width: {}%; background: {}", pct, color)
                            ></div>
                        </div>
                        <span class="payment-method-pct">{pct}"%"</span>
                    </div>
                }
            }).collect_view()}
        </div>
    }
}

/// Recent activity section.
#[component]
fn DashboardActivity() -> impl IntoView {
    view! {
        <div class="activity-section">
            <div class="activity-card">
                <div class="activity-header">
                    <h3 class="activity-title">"Recent payments"</h3>
                    <A href="/evm/payments" attr:class="activity-link">"View all"</A>
                </div>
                <RecentPayments />
            </div>

            <div class="activity-card">
                <div class="activity-header">
                    <h3 class="activity-title">"Network status"</h3>
                </div>
                <NetworkStatus />
            </div>
        </div>
    }
}

/// Recent payments list.
#[component]
fn RecentPayments() -> impl IntoView {
    let payments = vec![
        (
            "0x1a2b...3c4d",
            "0.5 ETH",
            "$892.50",
            "Completed",
            "2 min ago",
        ),
        (
            "0x5e6f...7g8h",
            "150 USDC",
            "$150.00",
            "Completed",
            "15 min ago",
        ),
        (
            "0x9i0j...1k2l",
            "0.25 ETH",
            "$446.25",
            "Processing",
            "32 min ago",
        ),
        (
            "0x3m4n...5o6p",
            "500 USDT",
            "$500.00",
            "Completed",
            "1 hour ago",
        ),
        (
            "0x7q8r...9s0t",
            "0.1 ETH",
            "$178.50",
            "Completed",
            "2 hours ago",
        ),
    ];

    view! {
        <div class="payments-list">
            {payments.into_iter().map(|(tx, amount, usd, status, time)| {
                let status_class = match status {
                    "Completed" => "badge badge-success",
                    "Processing" => "badge badge-warning",
                    _ => "badge badge-secondary",
                };
                view! {
                    <div class="payment-row">
                        <div class="payment-info">
                            <span class="payment-tx">{tx}</span>
                            <span class="payment-time">{time}</span>
                        </div>
                        <div class="payment-amount">
                            <span class="payment-crypto">{amount}</span>
                            <span class="payment-usd">{usd}</span>
                        </div>
                        <span class=status_class>{status}</span>
                    </div>
                }
            }).collect_view()}
        </div>
    }
}

/// Network status component.
#[component]
fn NetworkStatus() -> impl IntoView {
    let networks = vec![
        ("Ethereum", true, 12),
        ("Polygon", true, 128),
        ("Arbitrum", true, 1),
        ("Optimism", true, 1),
        ("Base", false, 0),
    ];

    view! {
        <div class="network-list">
            {networks.into_iter().map(|(name, connected, confirmations)| {
                view! {
                    <div class="network-row">
                        <div class="network-info">
                            <span class=if connected { "network-dot network-dot-online" } else { "network-dot network-dot-offline" }></span>
                            <span class="network-name">{name}</span>
                        </div>
                        <span class="network-confirmations">
                            {if connected {
                                format!("{} conf", confirmations)
                            } else {
                                "Offline".to_string()
                            }}
                        </span>
                    </div>
                }
            }).collect_view()}
        </div>
    }
}

// ============================================
// SVG Icons
// ============================================

#[component]
fn IconPlus() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="12" y1="5" x2="12" y2="19"></line>
            <line x1="5" y1="12" x2="19" y2="12"></line>
        </svg>
    }
}

#[component]
fn IconDownload() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"></path>
            <polyline points="7 10 12 15 17 10"></polyline>
            <line x1="12" y1="15" x2="12" y2="3"></line>
        </svg>
    }
}

#[component]
fn IconTrendUp() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="23 6 13.5 15.5 8.5 10.5 1 18"></polyline>
            <polyline points="17 6 23 6 23 12"></polyline>
        </svg>
    }
}

#[component]
fn IconTrendDown() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="23 18 13.5 8.5 8.5 13.5 1 6"></polyline>
            <polyline points="17 18 23 18 23 12"></polyline>
        </svg>
    }
}

#[component]
fn IconMinus() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="5" y1="12" x2="19" y2="12"></line>
        </svg>
    }
}
