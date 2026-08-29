//! Recent-activity row: the latest payments and per-chain network status.

use leptos::prelude::*;
use leptos_router::components::A;

/// Recent activity section.
#[component]
pub(super) fn DashboardActivity() -> impl IntoView {
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
