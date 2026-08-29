//! Key metrics row: the stats cards fed by the dashboard stats API.

use leptos::prelude::*;

use crate::api::EvmApiClient;
use crate::services::StatusUpdate;

use super::icons::{IconMinus, IconTrendDown, IconTrendUp};

/// Key metrics section — fetches real data from the dashboard stats API.
/// Re-fetches when a WebSocket InvoiceStatus or PaymentUpdate arrives.
#[component]
pub(super) fn DashboardMetrics() -> impl IntoView {
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
