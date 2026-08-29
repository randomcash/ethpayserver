//! Charts row: payment volume over time and the payment-method breakdown.

use leptos::prelude::*;

/// Charts section.
#[component]
pub(super) fn DashboardCharts() -> impl IntoView {
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
