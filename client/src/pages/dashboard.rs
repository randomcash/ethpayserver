//! Dashboard page - Stripe-inspired overview of EVM payment activity.

use crate::api::{DashboardAnalytics, EvmApiClient};
use crate::components::EmptyState;
use crate::services::StatusUpdate;
use leptos::prelude::*;
use leptos_router::components::A;

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
/// Re-fetches when a WebSocket InvoiceStatus or PaymentUpdate arrives.
#[component]
fn DashboardMetrics() -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");
    let ws_update = use_context::<ReadSignal<Option<StatusUpdate>>>();

    // Bump to trigger re-fetch when relevant WS messages arrive.
    let (ws_version, set_ws_version) = signal(0u32);
    if let Some(ws_update) = ws_update {
        Effect::new(move || {
            if let Some(StatusUpdate::InvoiceStatus { .. } | StatusUpdate::PaymentUpdate { .. }) =
                ws_update.get()
            {
                set_ws_version.update(|n| *n = n.wrapping_add(1));
            }
        });
    }

    let stats_resource = LocalResource::new(move || {
        let client = api.get();
        let _ = ws_version.get();
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

/// Charts section — one `/dashboard/analytics` fetch feeding both panels.
///
/// The volume chart and the methods breakdown are the same aggregation read
/// two ways, so they share a resource rather than each hitting the API.
/// Re-fetches on the same WebSocket messages as the metric cards.
#[component]
fn DashboardCharts() -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");
    let ws_update = use_context::<ReadSignal<Option<StatusUpdate>>>();

    let (ws_version, set_ws_version) = signal(0u32);
    if let Some(ws_update) = ws_update {
        Effect::new(move || {
            if let Some(StatusUpdate::InvoiceStatus { .. } | StatusUpdate::PaymentUpdate { .. }) =
                ws_update.get()
            {
                set_ws_version.update(|n| *n = n.wrapping_add(1));
            }
        });
    }

    // The 7D/30D/90D buttons used to be decorative. They now drive the window,
    // and the server rejects anything outside 1..=90.
    let (days, set_days) = signal(30u32);
    // None = follow the busiest asset the account actually uses; a click pins
    // one. Cleared whenever the window changes, since the busiest asset in a
    // different window may not be the pinned one.
    let (pinned_asset, set_pinned_asset) = signal(None::<String>);

    let analytics = LocalResource::new(move || {
        let client = api.get();
        let days = days.get();
        let _ = ws_version.get();
        async move { client.get_dashboard_analytics(days).await.ok() }
    });

    view! {
        <div class="charts-section">
            <div class="chart-card chart-card-main">
                <div class="chart-header">
                    <div>
                        <h3 class="chart-title">"Payment volume"</h3>
                        <p class="chart-subtitle">
                            {move || format!("Daily volume over the last {} days", days.get())}
                        </p>
                    </div>
                    <div class="chart-controls">
                        {[7_u32, 30, 90].into_iter().map(|window| {
                            view! {
                                <button
                                    class=move || if days.get() == window {
                                        "btn btn-ghost btn-xs active"
                                    } else {
                                        "btn btn-ghost btn-xs"
                                    }
                                    on:click=move |_| {
                                        set_days.set(window);
                                        set_pinned_asset.set(None);
                                    }
                                >
                                    {format!("{window}D")}
                                </button>
                            }
                        }).collect_view()}
                    </div>
                </div>
                <div class="chart-body">
                    <Suspense fallback=move || view! {
                        <p class="chart-subtitle">"Loading volume…"</p>
                    }>
                        {move || Suspend::new(async move {
                            match analytics.await {
                                Some(data) => view! {
                                    <VolumeChart
                                        data=data
                                        pinned_asset=pinned_asset
                                        set_pinned_asset=set_pinned_asset
                                    />
                                }.into_any(),
                                None => view! {
                                    <p class="chart-subtitle">
                                        "Volume is unavailable right now."
                                    </p>
                                }.into_any(),
                            }
                        })}
                    </Suspense>
                </div>
            </div>

            <div class="chart-card">
                <div class="chart-header">
                    <div>
                        <h3 class="chart-title">"Payment methods"</h3>
                        <p class="chart-subtitle">"Share of payments received"</p>
                    </div>
                </div>
                <div class="chart-body">
                    <Suspense fallback=move || view! {
                        <p class="chart-subtitle">"Loading breakdown…"</p>
                    }>
                        {move || Suspend::new(async move {
                            match analytics.await {
                                Some(data) => view! {
                                    <PaymentMethodsBreakdown data=data />
                                }.into_any(),
                                None => view! {
                                    <p class="chart-subtitle">
                                        "Breakdown is unavailable right now."
                                    </p>
                                }.into_any(),
                            }
                        })}
                    </Suspense>
                </div>
            </div>
        </div>
    }
}

/// Brand colour for the assets we ship payment methods for.
///
/// Anything else gets a neutral accent rather than a colour invented for it.
fn asset_color(symbol: &str) -> &'static str {
    match symbol {
        "ETH" | "WETH" => "#627eea",
        "USDC" => "#2775ca",
        "USDT" => "#26a17b",
        "DAI" | "xDAI" => "#f5ac37",
        "POL" | "MATIC" => "#8247e5",
        "WBTC" => "#f7931a",
        _ => "#6b7280",
    }
}

/// Parse a decimal amount string for bar scaling only.
///
/// The displayed figures always come from the server's exact strings; this is
/// used solely to work out how tall a bar is, where f64 is plenty.
fn amount_for_scale(amount: &str) -> f64 {
    amount.parse::<f64>().unwrap_or(0.0)
}

/// Daily volume for one asset.
///
/// Bars are per asset because volume is not summable across assets — 1 ETH
/// plus 1 USDC is not 2 of anything (RCS-225). The selected asset's symbol is
/// on the axis label so the numbers mean something.
#[component]
fn VolumeChart(
    data: DashboardAnalytics,
    pinned_asset: ReadSignal<Option<String>>,
    set_pinned_asset: WriteSignal<Option<String>>,
) -> impl IntoView {
    if data.assets.is_empty() {
        // An account with no payments gets an honest blank, not a flat line
        // and not the invented upward trend this panel used to draw.
        return view! {
            <EmptyState
                title="No payments yet"
                description="Volume will appear here once your first payment is received."
            />
        }
        .into_any();
    }

    let assets = StoredValue::new(data.assets);
    let selector = (assets.with_value(Vec::len) > 1).then(|| {
        let symbols: Vec<String> = assets.with_value(|a| {
            a.iter().map(|asset| asset.asset_symbol.clone()).collect()
        });
        view! {
            <div class="chart-controls">
                {symbols.into_iter().map(|symbol| {
                    let selected = symbol.clone();
                    let label = symbol.clone();
                    view! {
                        <button
                            class=move || {
                                let active = pinned_asset.get().as_deref() == Some(symbol.as_str());
                                if active { "btn btn-ghost btn-xs active" } else { "btn btn-ghost btn-xs" }
                            }
                            on:click=move |_| set_pinned_asset.set(Some(selected.clone()))
                        >
                            {label}
                        </button>
                    }
                }).collect_view()}
            </div>
        }
    });

    view! {
        <div class="volume-chart">
            {selector}
            {move || {
                let pinned = pinned_asset.get();
                let asset = assets.with_value(|list| {
                    pinned
                        .as_deref()
                        .and_then(|want| list.iter().find(|a| a.asset_symbol == want))
                        .or_else(|| list.first())
                        .cloned()
                });
                asset.map(|asset| {
                    // Scale to the busiest day so a quiet window still reads;
                    // a zero max would divide by zero, so it floors at 1.
                    let max = asset
                        .daily
                        .iter()
                        .map(|d| amount_for_scale(&d.amount))
                        .fold(0.0_f64, f64::max)
                        .max(f64::MIN_POSITIVE);
                    let color = asset_color(&asset.asset_symbol);
                    let last = asset.daily.len().saturating_sub(1);
                    let first_label = asset.daily.first().map(|d| d.date.clone()).unwrap_or_default();
                    let last_label = asset.daily.last().map(|d| d.date.clone()).unwrap_or_default();
                    let footer = format!(
                        "{} {} from {} payments",
                        asset.total_amount, asset.asset_symbol, asset.payment_count,
                    );
                    view! {
                        <div class="volume-chart-bars">
                            {asset.daily.iter().enumerate().map(|(i, point)| {
                                let height = (amount_for_scale(&point.amount) / max * 100.0)
                                    .clamp(0.0, 100.0);
                                let title = format!(
                                    "{}: {} {} ({} payments)",
                                    point.date, point.amount, asset.asset_symbol,
                                    point.payment_count,
                                );
                                let style = if i == last {
                                    format!("height: {height}%; background: {color}")
                                } else {
                                    format!("height: {height}%")
                                };
                                view! {
                                    <div
                                        class=if i == last { "volume-bar volume-bar-today" } else { "volume-bar" }
                                        style=style
                                        title=title
                                    ></div>
                                }
                            }).collect_view()}
                        </div>
                        <div class="volume-chart-labels">
                            <span>{first_label}</span>
                            <span>{footer}</span>
                            <span>{last_label}</span>
                        </div>
                    }
                })
            }}
        </div>
    }
    .into_any()
}

/// Payment methods breakdown.
///
/// Percentages are each asset's share of the *payment count* over the window.
/// Sharing by value would mean adding ETH to USDC, which is not a number.
#[component]
fn PaymentMethodsBreakdown(data: DashboardAnalytics) -> impl IntoView {
    if data.assets.is_empty() {
        return view! {
            <EmptyState
                title="No payments yet"
                description="The assets your customers pay with will appear here."
            />
        }
        .into_any();
    }

    view! {
        <div class="payment-methods">
            {data.assets.into_iter().map(|asset| {
                let color = asset_color(&asset.asset_symbol);
                let width = asset.share_percent.clamp(0.0, 100.0);
                let title = format!(
                    "{} payments totalling {} {}",
                    asset.payment_count, asset.total_amount, asset.asset_symbol,
                );
                view! {
                    <div class="payment-method-row" title=title>
                        <div class="payment-method-info">
                            <span class="payment-method-dot" style=format!("background: {color}")></span>
                            <span class="payment-method-name">{asset.asset_symbol}</span>
                        </div>
                        <div class="payment-method-bar-container">
                            <div
                                class="payment-method-bar"
                                style=format!("width: {width}%; background: {color}")
                            ></div>
                        </div>
                        <span class="payment-method-pct">{format!("{:.0}", asset.share_percent)}"%"</span>
                    </div>
                }
            }).collect_view()}
        </div>
    }
    .into_any()
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
