//! Payments page - Stripe-inspired payment history view.
//!
//! Uses types from `crate::api::types` which mirror the backend.

use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_params_map;

use crate::api::{ApiError, EvmApiClient, Payment};
use crate::app::StoreContext;

/// Helper to get chain name from chain ID.
fn chain_name(chain_id: u64) -> &'static str {
    match chain_id {
        1 => "Ethereum",
        137 => "Polygon",
        42161 => "Arbitrum",
        10 => "Optimism",
        8453 => "Base",
        56 => "BSC",
        43114 => "Avalanche",
        324 => "zkSync",
        59144 => "Linea",
        534352 => "Scroll",
        250 => "Fantom",
        100 => "Gnosis",
        11155111 => "Sepolia",
        _ => "Unknown",
    }
}

/// Helper to determine payment status display.
fn payment_status(payment: &Payment) -> &'static str {
    if payment.reorged {
        "reorged"
    } else if payment.confirmed_at.is_some() {
        "confirmed"
    } else {
        "pending"
    }
}

/// CSS class for payment status badge.
fn payment_status_class(payment: &Payment) -> &'static str {
    if payment.reorged {
        "badge badge-error"
    } else if payment.confirmed_at.is_some() {
        "badge badge-success"
    } else {
        "badge badge-warning"
    }
}

/// Format ISO date string for display.
fn format_date(iso: &str) -> String {
    if iso.len() >= 10 {
        let date_part = &iso[..10];
        let parts: Vec<&str> = date_part.split('-').collect();
        if parts.len() == 3 {
            let month = match parts[1] {
                "01" => "Jan",
                "02" => "Feb",
                "03" => "Mar",
                "04" => "Apr",
                "05" => "May",
                "06" => "Jun",
                "07" => "Jul",
                "08" => "Aug",
                "09" => "Sep",
                "10" => "Oct",
                "11" => "Nov",
                "12" => "Dec",
                _ => parts[1],
            };
            return format!("{} {}, {}", month, parts[2], parts[0]);
        }
    }
    iso.to_string()
}

/// Truncate address/hash for display.
fn truncate_hash(hash: &str, prefix: usize, suffix: usize) -> String {
    if hash.len() > prefix + suffix + 3 {
        format!("{}...{}", &hash[..prefix], &hash[hash.len() - suffix..])
    } else {
        hash.to_string()
    }
}

/// Number of payments per page.
const PAGE_SIZE: i64 = 20;

/// Payments list page.
#[component]
pub fn PaymentsPage() -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");
    let store_ctx = use_context::<StoreContext>().expect("StoreContext must be provided");

    // Filter state
    let (active_filter, set_active_filter) = signal("all".to_string());
    let (search_query, set_search_query) = signal(String::new());
    let (current_offset, set_current_offset) = signal(0i64);

    // Reset offset when filter or store changes
    let _reset_offset_on_filter = Effect::new(move || {
        let _ = active_filter.get();
        let _ = store_ctx.selected_store_id.get();
        set_current_offset.set(0);
    });

    // Refresh counter for manual re-fetch
    let (refresh, set_refresh) = signal(0u32);

    // Convert active filter to API status param
    let status_param = Signal::derive(move || match active_filter.get().as_str() {
        "all" => None,
        other => Some(other.to_string()),
    });

    let payments_resource = LocalResource::new(move || {
        let api = api.get();
        let store_id = store_ctx.selected_store_id.get();
        let status = status_param.get();
        let offset = current_offset.get();
        let _ = refresh.get();

        async move {
            let Some(sid) = store_id else {
                return Err(ApiError::Network("Please select a store first".to_string()));
            };
            api.list_payments(&sid, status.as_deref(), Some(PAGE_SIZE), Some(offset))
                .await
        }
    });

    let filters = vec![
        ("all", "All"),
        ("pending", "Pending"),
        ("confirmed", "Confirmed"),
    ];

    view! {
        <div class="payments-page">
            // Page Header
            <div class="page-header-row">
                <div>
                    <h1 class="page-title">"Payments"</h1>
                    <p class="page-description">"View all received payments across networks"</p>
                </div>
                <div class="page-actions">
                    <button class="btn btn-secondary btn-sm">
                        <IconExport />
                        "Export"
                    </button>
                </div>
            </div>

            // Filters and Search
            <div class="payments-toolbar">
                <div class="payments-filters">
                    {filters.into_iter().map(|(key, label)| {
                        let key_owned = key.to_string();
                        let key_for_click = key.to_string();
                        view! {
                            <button
                                class=move || if active_filter.get() == key_owned { "filter-tab active" } else { "filter-tab" }
                                on:click=move |_| set_active_filter.set(key_for_click.clone())
                            >
                                {label}
                            </button>
                        }
                    }).collect_view()}
                </div>

                <div class="payments-search">
                    <IconSearch />
                    <input
                        type="text"
                        placeholder="Search by tx hash, address..."
                        prop:value=move || search_query.get()
                        on:input=move |ev| set_search_query.set(event_target_value(&ev))
                    />
                </div>
            </div>

            // Content area with loading/error/data states
            <Suspense fallback=move || view! {
                <div class="loading-container">
                    <div class="loading-spinner"></div>
                    <p>"Loading payments..."</p>
                </div>
            }>
                {move || payments_resource.get().map(|result| match &*result {
                    Err(e) => view! {
                        <div class="error-container">
                            <p class="error-message">{e.to_string()}</p>
                            <button class="btn btn-secondary btn-sm" on:click=move |_| set_refresh.update(|n| *n += 1)>
                                "Retry"
                            </button>
                        </div>
                    }.into_any(),
                    Ok(response) => {
                        let total = response.total;
                        let payments = response.payments.clone();
                        let search = search_query.get();

                        // Client-side search filter
                        let filtered: Vec<Payment> = if search.is_empty() {
                            payments
                        } else {
                            let q = search.to_lowercase();
                            payments.into_iter().filter(|p| {
                                p.tx_hash.to_lowercase().contains(&q)
                                    || p.asset_symbol.to_lowercase().contains(&q)
                                    || p.from_address.as_ref()
                                        .map(|a| a.to_lowercase().contains(&q))
                                        .unwrap_or(false)
                                    || p.invoice_id.to_lowercase().contains(&q)
                            }).collect()
                        };

                        if filtered.is_empty() {
                            view! {
                                <div class="empty-state">
                                    <div class="empty-state-icon">
                                        <IconSearch />
                                    </div>
                                    <h3>"No payments found"</h3>
                                    <p>"Payments will appear here once invoices receive transactions."</p>
                                </div>
                            }.into_any()
                        } else {
                            let offset = current_offset.get();
                            let showing_start = offset + 1;
                            let showing_end = offset + filtered.len() as i64;
                            let has_prev = offset > 0;
                            let has_next = offset + PAGE_SIZE < total;

                            view! {
                                // Payment Table (Desktop)
                                <div class="payments-table-container desktop-only">
                                    <table class="payments-table">
                                        <thead>
                                            <tr>
                                                <th>"Transaction"</th>
                                                <th>"Amount"</th>
                                                <th>"Network"</th>
                                                <th>"Invoice"</th>
                                                <th>"Status"</th>
                                                <th>"Date"</th>
                                                <th></th>
                                            </tr>
                                        </thead>
                                        <tbody>
                                            {filtered.clone().into_iter().map(|payment| {
                                                view! { <PaymentRow payment=payment /> }
                                            }).collect_view()}
                                        </tbody>
                                    </table>
                                </div>

                                // Payment Cards (Mobile)
                                <div class="payments-cards mobile-only">
                                    {filtered.into_iter().map(|payment| {
                                        view! { <PaymentCard payment=payment /> }
                                    }).collect_view()}
                                </div>

                                // Pagination
                                <div class="payments-pagination">
                                    <div class="pagination-info">
                                        "Showing "<strong>{showing_start}"-"{showing_end}</strong>" of "<strong>{total}</strong>" payments"
                                    </div>
                                    <div class="pagination-controls">
                                        <button
                                            class="btn btn-ghost btn-sm"
                                            disabled=move || !has_prev
                                            on:click=move |_| set_current_offset.update(|o| *o = (*o - PAGE_SIZE).max(0))
                                        >"Previous"</button>
                                        <button
                                            class="btn btn-ghost btn-sm"
                                            disabled=move || !has_next
                                            on:click=move |_| set_current_offset.update(|o| *o += PAGE_SIZE)
                                        >"Next"</button>
                                    </div>
                                </div>
                            }.into_any()
                        }
                    }
                })}
            </Suspense>
        </div>
    }
}

/// Payment table row.
#[component]
fn PaymentRow(payment: Payment) -> impl IntoView {
    let tx_display = truncate_hash(&payment.tx_hash, 10, 8);
    let network = chain_name(payment.chain_id);
    let status = payment_status(&payment);
    let status_class = payment_status_class(&payment);
    let date_display = format_date(&payment.detected_at);
    let invoice_id = truncate_hash(&payment.invoice_id, 8, 4);
    let invoice_link = payment.invoice_id.clone();
    let payment_link = payment.id.clone();
    let amount_display = format!(
        "{} {}",
        format_crypto_amount(&payment.amount, &payment.asset_symbol),
        payment.asset_symbol
    );

    view! {
        <tr class="payment-row">
            <td>
                <div class="payment-tx-cell">
                    <code class="tx-hash">{tx_display}</code>
                    <button class="btn-icon-xs" title="View on explorer">
                        <IconExternalLink />
                    </button>
                </div>
            </td>
            <td>
                <span class="payment-amount">{amount_display}</span>
            </td>
            <td>
                <span class="payment-network">{network}</span>
            </td>
            <td>
                <A href=format!("/evm/invoices/{}", invoice_link) attr:class="payment-invoice-link">
                    {invoice_id}
                </A>
            </td>
            <td>
                <span class=status_class>{status}</span>
            </td>
            <td>
                <span class="payment-date">{date_display}</span>
            </td>
            <td>
                <A href=format!("/evm/payments/{}", payment_link) attr:class="btn btn-ghost btn-sm btn-icon">
                    <IconMore />
                </A>
            </td>
        </tr>
    }
}

/// Mobile payment card component.
#[component]
fn PaymentCard(payment: Payment) -> impl IntoView {
    let tx_display = truncate_hash(&payment.tx_hash, 8, 6);
    let network = chain_name(payment.chain_id);
    let status = payment_status(&payment);
    let status_class = payment_status_class(&payment);
    let date_display = format_date(&payment.detected_at);
    let invoice_id = truncate_hash(&payment.invoice_id, 8, 4);
    let invoice_link = payment.invoice_id.clone();
    let payment_link = payment.id.clone();
    let amount_display = format!(
        "{} {}",
        format_crypto_amount(&payment.amount, &payment.asset_symbol),
        payment.asset_symbol
    );

    view! {
        <div class="payment-card">
            <div class="payment-card-header">
                <div class="payment-card-amount">
                    <span class="payment-card-amount-value">{amount_display}</span>
                    <span class="payment-card-network">{network}</span>
                </div>
                <span class=status_class>{status}</span>
            </div>

            <div class="payment-card-details">
                <div class="payment-card-row">
                    <span class="payment-card-label">"Transaction"</span>
                    <div class="payment-card-tx">
                        <code>{tx_display}</code>
                        <IconExternalLink />
                    </div>
                </div>
                <div class="payment-card-row">
                    <span class="payment-card-label">"Invoice"</span>
                    <A href=format!("/evm/invoices/{}", invoice_link) attr:class="payment-card-invoice-link">
                        {invoice_id}
                    </A>
                </div>
            </div>

            <div class="payment-card-footer">
                <span class="payment-card-date">{date_display}</span>
                <A href=format!("/evm/payments/{}", payment_link) attr:class="btn btn-ghost btn-xs">
                    "Details"
                    <IconChevronRight />
                </A>
            </div>
        </div>
    }
}

/// Payment detail page.
#[component]
pub fn PaymentDetailPage() -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");
    let params = use_params_map();
    let payment_id = move || params.get().get("id").unwrap_or_default();

    let payment_resource = LocalResource::new(move || {
        let api = api.get();
        let id = payment_id();
        async move { api.get_payment(&id).await }
    });

    view! {
        <Suspense fallback=move || view! {
            <div class="loading-container">
                <div class="loading-spinner"></div>
                <p>"Loading payment..."</p>
            </div>
        }>
            {move || payment_resource.get().map(|result| match &*result {
                Err(e) => view! {
                    <div class="error-container">
                        <p class="error-message">{e.to_string()}</p>
                        <A href="/evm/payments" attr:class="btn btn-secondary btn-sm">"Back to payments"</A>
                    </div>
                }.into_any(),
                Ok(payment) => {
                    let payment = payment.clone();
                    view! { <PaymentDetailView payment=payment /> }.into_any()
                }
            })}
        </Suspense>
    }
}

/// Inner component that renders payment detail once data is loaded.
#[component]
fn PaymentDetailView(payment: Payment) -> impl IntoView {
    let network = chain_name(payment.chain_id);
    let status = payment_status(&payment);
    let status_class = payment_status_class(&payment);
    let amount_display = format!(
        "{} {}",
        format_crypto_amount(&payment.amount, &payment.asset_symbol),
        payment.asset_symbol
    );
    let detected_display = format_date(&payment.detected_at);
    let confirmed_display = payment
        .confirmed_at
        .as_ref()
        .map(|d| format_date(d))
        .unwrap_or_else(|| "Pending".to_string());
    let from_address = payment
        .from_address
        .clone()
        .unwrap_or_else(|| "Unknown".to_string());
    let block_number = payment
        .block_number
        .map(|b| b.to_string())
        .unwrap_or_else(|| "Pending".to_string());
    let invoice_id = payment.invoice_id.clone();
    let invoice_link = payment.invoice_id.clone();

    // Explorer URL based on chain
    let explorer_base = match payment.chain_id {
        1 => "https://etherscan.io",
        137 => "https://polygonscan.com",
        42161 => "https://arbiscan.io",
        10 => "https://optimistic.etherscan.io",
        8453 => "https://basescan.org",
        56 => "https://bscscan.com",
        43114 => "https://snowtrace.io",
        11155111 => "https://sepolia.etherscan.io",
        _ => "https://etherscan.io",
    };
    let tx_explorer_url = format!("{}/tx/{}", explorer_base, payment.tx_hash);
    let address_explorer_url = format!("{}/address/{}", explorer_base, from_address);

    view! {
        <div class="payment-detail-page">
            // Header
            <div class="payment-detail-header">
                <div class="payment-detail-header-left">
                    <A href="/evm/payments" attr:class="back-link">
                        <IconArrowLeft />
                        "Payments"
                    </A>
                    <div class="payment-detail-title-row">
                        <h1 class="payment-detail-title">{amount_display.clone()}</h1>
                        <span class=status_class>{status}</span>
                    </div>
                    <p class="payment-detail-subtitle">{network}" · "{detected_display.clone()}</p>
                </div>
                <div class="payment-detail-actions">
                    <a href=tx_explorer_url.clone() target="_blank" class="btn btn-secondary btn-sm">
                        <IconExternalLink />
                        "View on Explorer"
                    </a>
                </div>
            </div>

            // Content
            <div class="payment-detail-content">
                // Main Info Cards
                <div class="payment-detail-main">
                    // Transaction Details Card
                    <div class="detail-card">
                        <div class="detail-card-header">
                            <h3>"Transaction Details"</h3>
                        </div>
                        <div class="detail-card-body">
                            <div class="detail-row">
                                <span class="detail-label">"Transaction Hash"</span>
                                <div class="detail-value detail-value-mono">
                                    <span class="tx-hash-full">{payment.tx_hash.clone()}</span>
                                    <button class="btn-icon-xs" title="Copy">
                                        <IconCopy />
                                    </button>
                                </div>
                            </div>
                            <div class="detail-row">
                                <span class="detail-label">"Network"</span>
                                <span class="detail-value">{network}</span>
                            </div>
                            <div class="detail-row">
                                <span class="detail-label">"Block Number"</span>
                                <span class="detail-value">{block_number}</span>
                            </div>
                            <div class="detail-row">
                                <span class="detail-label">"From Address"</span>
                                <div class="detail-value detail-value-mono">
                                    <a href=address_explorer_url target="_blank" class="address-link">
                                        {truncate_hash(&from_address, 10, 8)}
                                    </a>
                                    <button class="btn-icon-xs" title="Copy">
                                        <IconCopy />
                                    </button>
                                </div>
                            </div>
                        </div>
                    </div>

                    // Payment Amount Card
                    <div class="detail-card">
                        <div class="detail-card-header">
                            <h3>"Payment Amount"</h3>
                        </div>
                        <div class="detail-card-body">
                            <div class="detail-row">
                                <span class="detail-label">"Amount Received"</span>
                                <span class="detail-value detail-value-lg">{amount_display}</span>
                            </div>
                            {payment.token_address.clone().map(|addr| view! {
                                <div class="detail-row">
                                    <span class="detail-label">"Token Contract"</span>
                                    <div class="detail-value detail-value-mono">
                                        <span>{truncate_hash(&addr, 10, 8)}</span>
                                        <button class="btn-icon-xs" title="Copy">
                                            <IconCopy />
                                        </button>
                                    </div>
                                </div>
                            })}
                        </div>
                    </div>

                    // Timeline Card
                    <div class="detail-card">
                        <div class="detail-card-header">
                            <h3>"Activity"</h3>
                        </div>
                        <div class="detail-card-body">
                            <div class="timeline">
                                {payment.confirmed_at.is_some().then(|| view! {
                                    <div class="timeline-item timeline-item-success">
                                        <div class="timeline-dot"></div>
                                        <div class="timeline-content">
                                            <span class="timeline-title">"Payment confirmed"</span>
                                            <span class="timeline-desc">"Reached required block confirmations"</span>
                                            <span class="timeline-time">{confirmed_display.clone()}</span>
                                        </div>
                                    </div>
                                })}
                                {(!payment.reorged && payment.confirmed_at.is_none()).then(|| view! {
                                    <div class="timeline-item timeline-item-pending">
                                        <div class="timeline-dot"></div>
                                        <div class="timeline-content">
                                            <span class="timeline-title">"Awaiting confirmation"</span>
                                            <span class="timeline-desc">"Waiting for block confirmations..."</span>
                                        </div>
                                    </div>
                                })}
                                {payment.reorged.then(|| view! {
                                    <div class="timeline-item timeline-item-error">
                                        <div class="timeline-dot"></div>
                                        <div class="timeline-content">
                                            <span class="timeline-title">"Payment invalidated"</span>
                                            <span class="timeline-desc">"Chain reorganization detected"</span>
                                        </div>
                                    </div>
                                })}
                                <div class="timeline-item">
                                    <div class="timeline-dot"></div>
                                    <div class="timeline-content">
                                        <span class="timeline-title">"Payment detected"</span>
                                        <span class="timeline-desc">"Transaction seen on chain"</span>
                                        <span class="timeline-time">{detected_display}</span>
                                    </div>
                                </div>
                            </div>
                        </div>
                    </div>
                </div>

                // Sidebar
                <div class="payment-detail-sidebar">
                    <div class="detail-card">
                        <div class="detail-card-header">
                            <h3>"Related Invoice"</h3>
                        </div>
                        <div class="detail-card-body">
                            <A href=format!("/evm/invoices/{}", invoice_link) attr:class="related-invoice-link">
                                <div class="related-invoice-info">
                                    <IconInvoice />
                                    <span>{truncate_hash(&invoice_id, 8, 4)}</span>
                                </div>
                                <IconChevronRight />
                            </A>
                        </div>
                    </div>

                    <div class="detail-card">
                        <div class="detail-card-header">
                            <h3>"Status"</h3>
                        </div>
                        <div class="detail-card-body">
                            <div class="status-info">
                                <span class=format!("{} status-badge-lg", status_class)>{status}</span>
                                {payment.reorged.then(|| view! {
                                    <p class="status-warning">
                                        "This payment was invalidated due to a chain reorganization."
                                    </p>
                                })}
                                {(!payment.reorged && payment.confirmed_at.is_none()).then(|| view! {
                                    <p class="status-info-text">
                                        "Waiting for block confirmations..."
                                    </p>
                                })}
                            </div>
                        </div>
                    </div>

                    <div class="detail-card">
                        <div class="detail-card-header">
                            <h3>"Raw Data"</h3>
                        </div>
                        <div class="detail-card-body">
                            <pre class="metadata-json">{format!(r#"{{
  "id": "{}",
  "chain_id": {},
  "invoice_id": "{}",
  "tx_hash": "{}",
  "amount": "{}",
  "asset_symbol": "{}",
  "block_number": {}
}}"#,
                                payment.id,
                                payment.chain_id,
                                payment.invoice_id,
                                payment.tx_hash,
                                payment.amount,
                                payment.asset_symbol,
                                payment.block_number.map(|b| b.to_string()).unwrap_or_else(|| "null".to_string())
                            )}</pre>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    }
}

/// Format crypto amount from wei/smallest unit to human readable.
fn format_crypto_amount(amount: &str, symbol: &str) -> String {
    // Get decimals based on token
    let decimals = match symbol {
        "ETH" | "POL" | "MATIC" => 18,
        "USDC" | "USDT" => 6,
        "DAI" => 18,
        _ => 18,
    };

    // Parse amount and convert
    if let Ok(val) = amount.parse::<u128>() {
        let divisor = 10u128.pow(decimals);
        let whole = val / divisor;
        let frac = val % divisor;

        if frac == 0 {
            return whole.to_string();
        }

        // Format with appropriate decimal places
        let frac_str = format!("{:0width$}", frac, width = decimals as usize);
        let trimmed = frac_str.trim_end_matches('0');
        if trimmed.is_empty() {
            whole.to_string()
        } else {
            format!("{}.{}", whole, trimmed)
        }
    } else {
        amount.to_string()
    }
}

// ============================================
// Icons
// ============================================

#[component]
fn IconExport() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"></path>
            <polyline points="17 8 12 3 7 8"></polyline>
            <line x1="12" y1="3" x2="12" y2="15"></line>
        </svg>
    }
}

#[component]
fn IconSearch() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="11" cy="11" r="8"></circle>
            <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
        </svg>
    }
}

#[component]
fn IconExternalLink() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"></path>
            <polyline points="15 3 21 3 21 9"></polyline>
            <line x1="10" y1="14" x2="21" y2="3"></line>
        </svg>
    }
}

#[component]
fn IconMore() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="1"></circle>
            <circle cx="19" cy="12" r="1"></circle>
            <circle cx="5" cy="12" r="1"></circle>
        </svg>
    }
}

#[component]
fn IconChevronRight() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="9 18 15 12 9 6"></polyline>
        </svg>
    }
}

#[component]
fn IconArrowLeft() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="19" y1="12" x2="5" y2="12"></line>
            <polyline points="12 19 5 12 12 5"></polyline>
        </svg>
    }
}

#[component]
fn IconCopy() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect>
            <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>
        </svg>
    }
}

#[component]
fn IconInvoice() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"></path>
            <polyline points="14 2 14 8 20 8"></polyline>
            <line x1="16" y1="13" x2="8" y2="13"></line>
            <line x1="16" y1="17" x2="8" y2="17"></line>
            <polyline points="10 9 9 9 8 9"></polyline>
        </svg>
    }
}
