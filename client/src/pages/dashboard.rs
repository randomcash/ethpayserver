//! Dashboard page - Stripe-inspired overview of EVM payment activity.

use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::*;
use leptos_router::components::A;
use send_wrapper::SendWrapper;

use crate::api::{ApiError, ChainHealthInfo, EvmApiClient};
use crate::app::{StoreContext, StoresStatus};
use crate::pages::payments::format::{
    format_crypto_amount, payment_status, payment_status_class, truncate_hash,
};
use crate::services::StatusUpdate;
use crate::util::{chain_name, relative_time};

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

/// How many payments the panel shows. The "View all" link covers the rest.
const RECENT_PAYMENTS_LIMIT: i64 = 5;

/// Recent payments list — the newest rows for the store the dashboard is
/// scoped to.
///
/// This panel used to be a literal list of invented tx hashes, amounts, dollar
/// values and timestamps, shown identically to every account including ones
/// with no payments at all (RCS-224). Everything here now comes off the row.
///
/// There is deliberately no fiat column. Rendering one needs a rate for the
/// asset *at the time the payment landed*; `/rates` only serves the current
/// rate, and today's price against a month-old payment is another invented
/// number next to a real one. The crypto amount and its chain are both true,
/// so that is what the row shows.
#[component]
fn RecentPayments() -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");
    let store_ctx = use_context::<StoreContext>().expect("StoreContext must be provided");
    // Signals off StoreContext are Copy; take them once so the resource closure
    // does not need to own the (non-Copy) context.
    let selected_store_id = store_ctx.selected_store_id;
    let store_status = store_ctx.stores_status;

    // Same WebSocket trigger DashboardMetrics uses, so the list does not go
    // stale next to counters that just moved.
    let (ws_version, set_ws_version) = signal(0u32);
    if let Some(ws_update) = use_context::<ReadSignal<Option<StatusUpdate>>>() {
        Effect::new(move || {
            if let Some(StatusUpdate::PaymentUpdate { .. }) = ws_update.get() {
                set_ws_version.update(|n| *n = n.wrapping_add(1));
            }
        });
    }

    // Relative timestamps are computed at render, so a dashboard left open
    // would otherwise say "Just now" indefinitely.
    let tick = use_tick(RELATIVE_TIME_TICK_MS);

    let payments_resource = LocalResource::new(move || {
        let api = api.get();
        let store_id = selected_store_id.get();
        let stores_loaded = matches!(store_status.get(), StoresStatus::Loaded);
        let _ = ws_version.get();

        async move {
            // Mirrors `pages/payments/list.rs` (RCS-171): "All Stores" is a
            // real query, but only once the store list has landed, and a
            // non-admin's 400 is a "pick a store" state rather than an error.
            // RCS-222 is widening the server side of that; when it lands this
            // branch simply stops being reached.
            if store_id.is_none() && !stores_loaded {
                return Ok(None);
            }
            match api
                .list_payments(
                    store_id.as_deref(),
                    None,
                    Some(RECENT_PAYMENTS_LIMIT),
                    Some(0),
                )
                .await
            {
                Ok(response) => Ok(Some(response)),
                Err(ApiError::Http { status: 400, .. }) if store_id.is_none() => Ok(None),
                Err(e) => Err(e),
            }
        }
    });

    view! {
        <Suspense fallback=move || view! {
            <div class="activity-note">"Loading payments…"</div>
        }>
            {move || payments_resource.get().map(|result| match &*result {
                Err(e) => view! {
                    <div class="activity-note activity-note-error">
                        {format!("Could not load payments: {e}")}
                    </div>
                }.into_any(),
                // Store list still settling, or an "All Stores" read this
                // account is not allowed to make. Either way we have no rows
                // for a specific store, and inventing some is the bug.
                Ok(None) => view! {
                    <div class="activity-note">
                        "Select a store in the sidebar to see its payments."
                    </div>
                }.into_any(),
                Ok(Some(response)) if response.payments.is_empty() => view! {
                    <div class="activity-note">
                        "No payments yet. They appear here once an invoice receives a transaction."
                    </div>
                }.into_any(),
                Ok(Some(response)) => {
                    // Server orders by detected_at DESC, so these are already
                    // newest first.
                    let rows = response.payments.clone();
                    // One clock read for the whole list, re-read on each tick.
                    let _ = tick.get();
                    let now_ms = js_sys::Date::now();

                    view! {
                        <div class="payments-list">
                            {rows.into_iter().map(|payment| {
                                let tx = truncate_hash(&payment.tx_hash, 8, 6);
                                let when = relative_time(&payment.detected_at, now_ms)
                                    .unwrap_or_else(|| payment.detected_at.clone());
                                let amount = format!(
                                    "{} {}",
                                    format_crypto_amount(&payment.amount, payment.decimals),
                                    payment.asset_symbol
                                );
                                let network = chain_name(payment.chain_id);
                                let status = payment_status(&payment);
                                let status_class = payment_status_class(&payment);
                                let href = format!("/evm/payments/{}", payment.id);

                                view! {
                                    <A href=href attr:class="payment-row">
                                        <div class="payment-info">
                                            <span class="payment-tx">{tx}</span>
                                            <span class="payment-time">{when}</span>
                                        </div>
                                        <div class="payment-amount">
                                            <span class="payment-crypto">{amount}</span>
                                            <span class="payment-chain">{network}</span>
                                        </div>
                                        <span class=status_class>{status}</span>
                                    </A>
                                }
                            }).collect_view()}
                        </div>
                    }.into_any()
                }
            })}
        </Suspense>
    }
}

/// The connection state a monitor reports for one chain.
///
/// `/health/chains` sends this as a free-form string, and the failure case
/// carries its reason inline ("failed: no RPC endpoint configured"), so this
/// classifies rather than deserialises. Anything unrecognised is [`Self::Unknown`]
/// and shows as such: guessing "connected" for a string we cannot read is how
/// the panel would go green over a monitor that is not running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChainState {
    Connected,
    Connecting,
    Disconnected,
    Failed,
    Unknown,
}

impl ChainState {
    fn from_status(status: &str) -> Self {
        let status = status.trim();
        if status.eq_ignore_ascii_case("connected") {
            Self::Connected
        } else if status.eq_ignore_ascii_case("connecting") {
            Self::Connecting
        } else if status.eq_ignore_ascii_case("disconnected") {
            Self::Disconnected
        } else if status
            .as_bytes()
            // Byte slice, not `&status[..6]`: a status whose first six bytes
            // land mid-character would panic on a str slice and take the whole
            // dashboard down with it.
            .get(..6)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"failed"))
        {
            Self::Failed
        } else {
            Self::Unknown
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Connected => "Connected",
            Self::Connecting => "Connecting",
            Self::Disconnected => "Disconnected",
            Self::Failed => "Failed",
            Self::Unknown => "Unknown",
        }
    }

    /// Dot class for this state.
    ///
    /// `is_healthy` is the monitor's own verdict and only matters while
    /// connected: a monitor can hold an RPC connection and still not be
    /// processing, which is precisely the state RCS-196 rendered as green.
    fn dot_class(self, is_healthy: bool) -> &'static str {
        match self {
            Self::Connected if is_healthy => "network-dot network-dot-online",
            Self::Connected | Self::Connecting => "network-dot network-dot-degraded",
            Self::Disconnected => "network-dot network-dot-offline",
            Self::Failed | Self::Unknown => "network-dot network-dot-error",
        }
    }
}

/// How far behind the chain head the monitor is, when both numbers are known.
fn monitor_lag(chain: &ChainHealthInfo) -> Option<u64> {
    let head = chain.current_block?;
    let processed = chain.last_processed_block?;
    Some(head.saturating_sub(processed))
}

/// Right-hand text for a chain row.
///
/// Replaces the invented "N conf" number: monitor lag is derived from two
/// values the monitor actually publishes, and reads as "we do not know" when
/// either is missing.
fn chain_detail(chain: &ChainHealthInfo, state: ChainState) -> String {
    if state != ChainState::Connected {
        return state.label().to_string();
    }
    match monitor_lag(chain) {
        None => "no block data".to_string(),
        Some(0) => "in sync".to_string(),
        Some(1) => "1 block behind".to_string(),
        Some(n) => format!("{n} blocks behind"),
    }
}

/// Display name for a chain row, preferring what the monitor reported.
fn chain_label(chain: &ChainHealthInfo) -> String {
    if chain.chain_name.trim().is_empty() {
        chain_name(chain.chain_id).to_string()
    } else {
        chain.chain_name.clone()
    }
}

/// Network status panel — the chains the monitor is actually reporting on.
///
/// This was a hardcoded five-chain list that named chains which are not
/// enabled and showed them all connected with invented confirmation counts
/// (RCS-223). It would have stayed green throughout RCS-196, where every
/// monitor RPC endpoint was empty. Nothing here has a default: no data means
/// the panel says so.
#[component]
fn NetworkStatus() -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");

    // The monitor republishes every 10s under a 60s TTL, so a panel rendered
    // once and never refreshed is a stale claim about live infrastructure.
    // `/health` is exempt from the IP rate limit tiers, so polling is safe.
    let tick = use_tick(CHAIN_HEALTH_POLL_MS);

    let health_resource = LocalResource::new(move || {
        let api = api.get();
        let _ = tick.get();
        async move { api.get_chains_health().await }
    });

    view! {
        <Suspense fallback=move || view! {
            <div class="activity-note">"Loading chain status…"</div>
        }>
            {move || health_resource.get().map(|result| match &*result {
                // 503 means the monitor has published no health at all. That is
                // "we cannot tell you", which is not the same as "all good" and
                // must not render as rows - a green panel over dead monitors
                // was RCS-196.
                //
                // There is no 403 case: the endpoint answers everyone, and
                // simply says less to a non-admin (block heights, the watched
                // address count and the failure reason are withheld). Whether a
                // chain is up is exactly what a merchant needs.
                Err(ApiError::Http { status: 503, .. }) => view! {
                    <div class="activity-note activity-note-error">
                        "No chain health reported — the monitor has not published any."
                    </div>
                }.into_any(),
                Err(e) => view! {
                    <div class="activity-note activity-note-error">
                        {format!("Could not load chain status: {e}")}
                    </div>
                }.into_any(),
                Ok(response) if response.chains.is_empty() => view! {
                    <div class="activity-note activity-note-error">
                        "No chain health reported — the monitor has not published any."
                    </div>
                }.into_any(),
                Ok(response) => {
                    let stale = !response.data_fresh;
                    let chains = response.chains.clone();

                    view! {
                        {stale.then(|| view! {
                            <div class="activity-note activity-note-error">
                                "Health data is stale — the monitor has stopped reporting."
                            </div>
                        })}
                        <div class="network-list">
                            {chains.into_iter().map(|chain| {
                                let state = ChainState::from_status(&chain.status);
                                let dot_class = state.dot_class(chain.is_healthy);
                                let name = chain_label(&chain);
                                let detail = chain_detail(&chain, state);
                                // The raw status carries the failure reason;
                                // keep it reachable without widening the row.
                                let title = format!(
                                    "chain {} — {}",
                                    chain.chain_id,
                                    chain.status
                                );

                                view! {
                                    <div class="network-row" title=title>
                                        <div class="network-info">
                                            <span class=dot_class></span>
                                            <span class="network-name">{name}</span>
                                        </div>
                                        <span class="network-detail">{detail}</span>
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    }.into_any()
                }
            })}
        </Suspense>
    }
}

/// How often the network panel re-reads `/health/chains`, in milliseconds.
/// Well inside the 60s TTL on the monitor's Redis keys.
const CHAIN_HEALTH_POLL_MS: u32 = 20_000;

/// How often rendered relative timestamps are recomputed, in milliseconds.
const RELATIVE_TIME_TICK_MS: u32 = 30_000;

/// A counter that increments every `interval_ms` for as long as the calling
/// component is alive.
///
/// The interval handle is dropped in `on_cleanup`: a timer that outlives its
/// owner and writes a disposed signal panics the whole app, which was RCS-220.
fn use_tick(interval_ms: u32) -> ReadSignal<u32> {
    let (tick, set_tick) = signal(0u32);

    let handle: Rc<RefCell<Option<gloo_timers::callback::Interval>>> = Rc::new(RefCell::new(None));
    let handle_for_effect = handle.clone();
    Effect::new(move |_| {
        let interval = gloo_timers::callback::Interval::new(interval_ms, move || {
            set_tick.update(|n| *n = n.wrapping_add(1));
        });
        *handle_for_effect.borrow_mut() = Some(interval);
    });

    let handle_for_cleanup = SendWrapper::new(handle);
    on_cleanup(move || {
        handle_for_cleanup.borrow_mut().take();
    });

    tick
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

#[cfg(test)]
mod tests {
    use super::{ChainState, chain_detail, chain_label, monitor_lag};
    use crate::api::ChainHealthInfo;

    fn chain(status: &str, current: Option<u64>, processed: Option<u64>) -> ChainHealthInfo {
        ChainHealthInfo {
            chain_id: 11_155_111,
            chain_name: "Sepolia".to_string(),
            status: status.to_string(),
            current_block: current,
            last_processed_block: processed,
            watched_addresses: None,
            is_healthy: true,
        }
    }

    #[test]
    fn classifies_the_statuses_the_server_sends() {
        assert_eq!(ChainState::from_status("connected"), ChainState::Connected);
        assert_eq!(
            ChainState::from_status("connecting"),
            ChainState::Connecting
        );
        assert_eq!(
            ChainState::from_status("disconnected"),
            ChainState::Disconnected
        );
    }

    #[test]
    fn a_failure_keeps_its_reason_and_still_classifies() {
        // The server formats this variant as "failed: {msg}", so an equality
        // check against "failed" would fall through to Unknown.
        assert_eq!(
            ChainState::from_status("failed: no RPC endpoint configured"),
            ChainState::Failed
        );
        assert_eq!(ChainState::from_status("failed"), ChainState::Failed);
    }

    #[test]
    fn an_unreadable_status_is_never_treated_as_connected() {
        assert_eq!(ChainState::from_status(""), ChainState::Unknown);
        assert_eq!(ChainState::from_status("weird"), ChainState::Unknown);
        assert_eq!(ChainState::from_status("fail"), ChainState::Unknown);
        // Not a status the server sends, but a str slice at byte 6 here would
        // panic rather than classify.
        assert_eq!(ChainState::from_status("ééééé"), ChainState::Unknown);
    }

    #[test]
    fn only_a_healthy_connection_gets_the_green_dot() {
        let online = "network-dot network-dot-online";
        assert_eq!(ChainState::Connected.dot_class(true), online);
        // Connected but the monitor itself says unhealthy: RCS-196.
        assert_ne!(ChainState::Connected.dot_class(false), online);
        assert_ne!(ChainState::Connecting.dot_class(true), online);
        assert_ne!(ChainState::Disconnected.dot_class(true), online);
        assert_ne!(ChainState::Failed.dot_class(true), online);
        assert_ne!(ChainState::Unknown.dot_class(true), online);
    }

    #[test]
    fn lag_is_the_gap_between_head_and_processed() {
        assert_eq!(
            monitor_lag(&chain("connected", Some(100), Some(97))),
            Some(3)
        );
        assert_eq!(
            monitor_lag(&chain("connected", Some(100), Some(100))),
            Some(0)
        );
        // A processed block ahead of the head is a race, not a negative lag.
        assert_eq!(
            monitor_lag(&chain("connected", Some(100), Some(101))),
            Some(0)
        );
    }

    #[test]
    fn missing_block_numbers_have_no_lag_rather_than_zero() {
        assert_eq!(monitor_lag(&chain("connected", None, Some(97))), None);
        assert_eq!(monitor_lag(&chain("connected", Some(100), None)), None);
        assert_eq!(
            chain_detail(&chain("connected", None, None), ChainState::Connected),
            "no block data"
        );
    }

    #[test]
    fn detail_reads_the_state_when_not_connected() {
        assert_eq!(
            chain_detail(
                &chain("failed: rpc timeout", Some(100), Some(97)),
                ChainState::Failed
            ),
            "Failed"
        );
        assert_eq!(
            chain_detail(&chain("connecting", None, None), ChainState::Connecting),
            "Connecting"
        );
    }

    #[test]
    fn detail_pluralises_blocks() {
        assert_eq!(
            chain_detail(
                &chain("connected", Some(100), Some(100)),
                ChainState::Connected
            ),
            "in sync"
        );
        assert_eq!(
            chain_detail(
                &chain("connected", Some(100), Some(99)),
                ChainState::Connected
            ),
            "1 block behind"
        );
        assert_eq!(
            chain_detail(
                &chain("connected", Some(100), Some(90)),
                ChainState::Connected
            ),
            "10 blocks behind"
        );
    }

    #[test]
    fn falls_back_to_the_chain_id_when_the_monitor_sends_no_name() {
        let mut c = chain("connected", Some(1), Some(1));
        assert_eq!(chain_label(&c), "Sepolia");
        c.chain_name = "  ".to_string();
        c.chain_id = 1;
        assert_eq!(chain_label(&c), "Ethereum");
    }
}
