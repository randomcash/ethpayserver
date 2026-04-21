//! Invoice management pages - Stripe-inspired design.
//!
//! Uses types from `crate::api::types` which mirror the backend.

use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_params_map;

use crate::api::{ApiError, EvmApiClient, Invoice, InvoiceStatusExt, Payment};
use crate::app::StoreContext;
use crate::components::CreateInvoiceSignal;
use crate::services::StatusUpdate;

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

/// Helper to determine payment status from confirmed_at.
fn payment_status(payment: &Payment) -> &'static str {
    if payment.reorged {
        "reorged"
    } else if payment.confirmed_at.is_some() {
        "confirmed"
    } else {
        "confirming"
    }
}

/// CSS class for payment status.
fn payment_status_class(payment: &Payment) -> &'static str {
    if payment.reorged {
        "badge badge-error"
    } else if payment.confirmed_at.is_some() {
        "badge badge-success"
    } else {
        "badge badge-warning"
    }
}

/// Number of invoices per page.
const PAGE_SIZE: i64 = 20;

/// Invoice list page.
#[component]
pub fn InvoicesPage() -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");
    let store_ctx = use_context::<StoreContext>().expect("StoreContext must be provided");

    // Filter state
    let (active_filter, set_active_filter) = signal("all".to_string());
    let (currency_filter, set_currency_filter) = signal("all".to_string());
    let (search_query, set_search_query) = signal(String::new());
    let (current_offset, set_current_offset) = signal(0i64);

    // Reset offset when any filter or store changes
    let _reset_offset_on_filter = Effect::new(move || {
        let _ = active_filter.get();
        let _ = currency_filter.get();
        let _ = store_ctx.selected_store_id.get();
        set_current_offset.set(0);
    });

    // Refresh counter for manual re-fetch
    let (refresh, set_refresh) = signal(0u32);

    // WebSocket-driven status patches applied without full re-fetch.
    // Maps invoice_id -> new status string from the most recent WS event.
    let (ws_patches, set_ws_patches) = signal(std::collections::HashMap::<String, String>::new());
    let ws_update = use_context::<ReadSignal<Option<StatusUpdate>>>();
    if let Some(ws_update) = ws_update {
        Effect::new(move || {
            if let Some(StatusUpdate::InvoiceStatus {
                ref invoice_id,
                ref status,
            }) = ws_update.get()
            {
                set_ws_patches.update(|patches| {
                    patches.insert(invoice_id.clone(), status.clone());
                });
            }
        });
    }
    // Clear patches when a fresh fetch completes (the fetched data is authoritative).
    Effect::new(move || {
        let _ = refresh.get();
        set_ws_patches.update(|patches| patches.clear());
    });

    // Use the shared create-invoice modal signal
    let create_invoice_signal =
        use_context::<CreateInvoiceSignal>().expect("CreateInvoiceSignal must be provided");

    // Refresh the list when the modal closes (invoice may have been created)
    let was_showing = StoredValue::new(false);
    Effect::new(move || {
        let showing = create_invoice_signal.show.get();
        if was_showing.get_value() && !showing {
            set_refresh.update(|n| *n += 1);
        }
        was_showing.set_value(showing);
    });

    // Convert filters to API params
    let status_param = Signal::derive(move || match active_filter.get().as_str() {
        "all" => None,
        other => Some(other.to_string()),
    });
    let currency_param = Signal::derive(move || match currency_filter.get().as_str() {
        "all" => None,
        other => Some(other.to_string()),
    });

    let invoices_resource = LocalResource::new(move || {
        let api = api.get();
        let store_id = store_ctx.selected_store_id.get();
        let status = status_param.get();
        let currency = currency_param.get();
        let offset = current_offset.get();
        let _ = refresh.get();

        async move {
            let Some(sid) = store_id else {
                return Err(ApiError::Network("Please select a store first".to_string()));
            };
            api.list_invoices(
                &sid,
                status.as_deref(),
                currency.as_deref(),
                Some(PAGE_SIZE),
                Some(offset),
            )
            .await
        }
    });

    view! {
        <div class="invoices-page">
            // Page Header
            <div class="page-header-row">
                <div>
                    <h1 class="page-title">"Invoices"</h1>
                    <p class="page-description">"Create and manage payment invoices"</p>
                </div>
                <div class="page-actions">
                    <button class="btn btn-secondary btn-sm">
                        <IconExport />
                        "Export"
                    </button>
                    <button class="btn btn-primary btn-sm" on:click=move |_| create_invoice_signal.open()>
                        <IconPlus />
                        "Create invoice"
                    </button>
                </div>
            </div>

            // Filters and Search
            <div class="invoices-toolbar">
                <div class="invoices-filters">
                    <select
                        class="filter-select"
                        on:change=move |ev| set_active_filter.set(event_target_value(&ev))
                        prop:value=move || active_filter.get()
                    >
                        <option value="all">"All statuses"</option>
                        <option value="pending">"Pending"</option>
                        <option value="processing">"Processing"</option>
                        <option value="partially_paid">"Partially Paid"</option>
                        <option value="paid">"Paid"</option>
                        <option value="expired">"Expired"</option>
                        <option value="cancelled">"Cancelled"</option>
                        <option value="refunded">"Refunded"</option>
                        <option value="late_paid">"Late Paid"</option>
                    </select>

                    <select
                        class="filter-select"
                        on:change=move |ev| set_currency_filter.set(event_target_value(&ev))
                        prop:value=move || currency_filter.get()
                    >
                        <option value="all">"All currencies"</option>
                        <option value="USD">"USD"</option>
                        <option value="EUR">"EUR"</option>
                        <option value="GBP">"GBP"</option>
                        <option value="BTC">"BTC"</option>
                        <option value="ETH">"ETH"</option>
                        <option value="USDT">"USDT"</option>
                        <option value="USDC">"USDC"</option>
                        <option value="DAI">"DAI"</option>
                    </select>
                </div>

                <div class="invoices-search">
                    <IconSearch />
                    <input
                        type="text"
                        placeholder="Search invoices..."
                        prop:value=move || search_query.get()
                        on:input=move |ev| set_search_query.set(event_target_value(&ev))
                    />
                </div>
            </div>

            // Content area with loading/error/data states
            <Suspense fallback=move || view! {
                <div class="loading-container">
                    <div class="loading-spinner"></div>
                    <p>"Loading invoices..."</p>
                </div>
            }>
                {move || invoices_resource.get().map(|result| match &*result {
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
                        let mut invoices = response.invoices.clone();
                        let search = search_query.get();

                        // Apply WebSocket status patches in-place (avoids full re-fetch).
                        let patches = ws_patches.get();
                        if !patches.is_empty() {
                            for invoice in &mut invoices {
                                if let Some(new_status) = patches.get(&invoice.id)
                                    && let Ok(parsed) = serde_json::from_value(
                                        serde_json::Value::String(new_status.clone()),
                                    )
                                {
                                    invoice.status = parsed;
                                }
                            }
                        }

                        // Client-side search filter
                        let filtered: Vec<Invoice> = if search.is_empty() {
                            invoices
                        } else {
                            let q = search.to_lowercase();
                            invoices.into_iter().filter(|inv| {
                                inv.id.to_lowercase().contains(&q)
                                    || inv.currency.to_lowercase().contains(&q)
                                    || inv.amount.contains(&q)
                                    || inv.metadata.as_ref()
                                        .map(|m| m.to_string().to_lowercase().contains(&q))
                                        .unwrap_or(false)
                            }).collect()
                        };

                        if filtered.is_empty() {
                            view! {
                                <div class="empty-state">
                                    <div class="empty-state-icon">
                                        <IconSearch />
                                    </div>
                                    <h3>"No invoices found"</h3>
                                    <p>"Create an invoice to get started, or adjust your filters."</p>
                                </div>
                            }.into_any()
                        } else {
                            let offset = current_offset.get();
                            let showing_start = offset + 1;
                            let showing_end = offset + filtered.len() as i64;
                            let has_prev = offset > 0;
                            let has_next = offset + PAGE_SIZE < total;

                            view! {
                                // Invoice Table (Desktop)
                                <div class="invoices-table-container desktop-only">
                                    <table class="invoices-table">
                                        <thead>
                                            <tr>
                                                <th>"Invoice"</th>
                                                <th>"Amount"</th>
                                                <th>"Received"</th>
                                                <th>"Status"</th>
                                                <th>"Created"</th>
                                                <th></th>
                                            </tr>
                                        </thead>
                                        <tbody>
                                            {filtered.clone().into_iter().map(|invoice| {
                                                view! { <InvoiceRow invoice=invoice /> }
                                            }).collect_view()}
                                        </tbody>
                                    </table>
                                </div>

                                // Invoice Cards (Mobile)
                                <div class="invoices-cards mobile-only">
                                    {filtered.into_iter().map(|invoice| {
                                        view! { <InvoiceCard invoice=invoice /> }
                                    }).collect_view()}
                                </div>

                                // Pagination
                                <div class="invoices-pagination">
                                    <div class="pagination-info">
                                        "Showing "<strong>{showing_start}"-"{showing_end}</strong>" of "<strong>{total}</strong>" invoices"
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

            // Create Invoice Modal is rendered at the layout level (app.rs)
            // via the shared CreateInvoiceSignal context.
        </div>
    }
}

/// Helper to extract a field from invoice metadata.
fn get_metadata_field(invoice: &Invoice, field: &str) -> Option<String> {
    invoice
        .metadata
        .as_ref()
        .and_then(|m: &serde_json::Value| m.get(field))
        .and_then(|v: &serde_json::Value| v.as_str())
        .map(|s: &str| s.to_string())
}

/// Format ISO date string for display.
fn format_date(iso: &str) -> String {
    // Simple formatting - in production would use chrono
    if iso.len() >= 10 {
        let date_part = &iso[..10];
        // Parse YYYY-MM-DD and format as "Jan DD, YYYY"
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

/// Format an amount with its currency (e.g., "$100.00 USD", "0.5 ETH").
fn format_amount(amount: &str, currency: &str) -> String {
    match currency {
        "USD" => format!("${} {}", amount, currency),
        "EUR" => format!("\u{20ac}{} {}", amount, currency),
        "GBP" => format!("\u{00a3}{} {}", amount, currency),
        _ => format!("{} {}", amount, currency),
    }
}

/// Invoice table row.
#[component]
fn InvoiceRow(invoice: Invoice) -> impl IntoView {
    let invoice_id = invoice.id.clone();
    let invoice_link = invoice.id.clone();
    let order_id = get_metadata_field(&invoice, "order_id");
    let created_display = format_date(&invoice.created_at);

    view! {
        <tr class="invoice-row">
            <td>
                <div class="invoice-id-cell">
                    <A href=format!("/evm/invoices/{}", invoice_link) attr:class="invoice-id-link">
                        {invoice_id}
                    </A>
                    {order_id.map(|oid| view! {
                        <span class="invoice-order-id">{oid}</span>
                    })}
                </div>
            </td>
            <td>
                <div class="invoice-amount-cell">
                    <span class="invoice-fiat">{format_amount(&invoice.amount, &invoice.currency)}</span>
                </div>
            </td>
            <td>
                <div class="invoice-amount-cell">
                    <span class="invoice-received">{format_amount(&invoice.amount_received, &invoice.currency)}</span>
                </div>
            </td>
            <td>
                <span class=invoice.status.css_class()>{invoice.status.label()}</span>
            </td>
            <td>
                <span class="invoice-date">{created_display}</span>
            </td>
            <td>
                <button class="btn btn-ghost btn-sm btn-icon">
                    <IconMore />
                </button>
            </td>
        </tr>
    }
}

/// Mobile invoice card component.
#[component]
fn InvoiceCard(invoice: Invoice) -> impl IntoView {
    let invoice_id = invoice.id.clone();
    let invoice_link = invoice.id.clone();
    let order_id = get_metadata_field(&invoice, "order_id");
    let created_display = format_date(&invoice.created_at);

    view! {
        <A href=format!("/evm/invoices/{}", invoice_link) attr:class="invoice-card">
            <div class="invoice-card-header">
                <div class="invoice-card-id">
                    <span class="invoice-card-id-text">{invoice_id}</span>
                    {order_id.map(|oid| view! {
                        <span class="invoice-card-order">{oid}</span>
                    })}
                </div>
                <span class=invoice.status.css_class()>{invoice.status.label()}</span>
            </div>
            <div class="invoice-card-amounts">
                <div class="invoice-card-amount">
                    <span class="invoice-card-label">"Amount"</span>
                    <span class="invoice-card-value">{format_amount(&invoice.amount, &invoice.currency)}</span>
                </div>
                <div class="invoice-card-amount">
                    <span class="invoice-card-label">"Received"</span>
                    <span class="invoice-card-value">{format_amount(&invoice.amount_received, &invoice.currency)}</span>
                </div>
            </div>
            <div class="invoice-card-footer">
                <span class="invoice-card-date">{created_display}</span>
                <IconChevronRight />
            </div>
        </A>
    }
}

/// Chevron right icon for mobile cards.
#[component]
fn IconChevronRight() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="9 18 15 12 9 6"></polyline>
        </svg>
    }
}

/// Invoice detail page - fetches from GET /invoices/{id} and GET /invoices/{id}/payments.
#[component]
pub fn InvoiceDetailPage() -> impl IntoView {
    let params = use_params_map();
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");

    let invoice_id = Signal::derive(move || params.get().get("id").unwrap_or_default());

    // Refresh counter for retry on error
    let (refresh, set_refresh) = signal(0u32);

    let detail_resource = LocalResource::new(move || {
        let api = api.get();
        let id = invoice_id.get();
        let _ = refresh.get();
        async move {
            if id.is_empty() {
                return Err(ApiError::Network("No invoice ID".to_string()));
            }
            let invoice = api.get_invoice(&id).await?;
            let payments = api.get_invoice_payments(&id).await?;
            Ok((invoice, payments))
        }
    });

    view! {
        <div class="invoice-detail-page">
            <Suspense fallback=move || view! {
                <div class="loading-container">
                    <div class="loading-spinner"></div>
                    <p>"Loading invoice..."</p>
                </div>
            }>
                {move || detail_resource.get().map(|result| match &*result {
                    Err(e) => view! {
                        <div class="error-container">
                            <p class="error-message">{e.to_string()}</p>
                            <button class="btn btn-secondary btn-sm" on:click=move |_| set_refresh.update(|n| *n += 1)>
                                "Retry"
                            </button>
                        </div>
                    }.into_any(),
                    Ok((invoice, payments)) => {
                        view! {
                            <InvoiceDetailContent invoice=invoice.clone() payments=payments.clone() />
                        }.into_any()
                    }
                })}
            </Suspense>
        </div>
    }
}

/// Count confirmed (non-reorged) payments.
fn confirmed_payment_count(payments: &[Payment]) -> usize {
    payments
        .iter()
        .filter(|p| p.confirmed_at.is_some() && !p.reorged)
        .count()
}

/// Truncate a hex string for display (e.g., "0x1234abcd...5678ef01").
fn truncate_hex(s: &str, prefix_len: usize, suffix_len: usize) -> String {
    if s.len() > prefix_len + suffix_len + 3 {
        format!("{}...{}", &s[..prefix_len], &s[s.len() - suffix_len..])
    } else {
        s.to_string()
    }
}

/// Inner content of the invoice detail page (rendered after data loads).
#[component]
fn InvoiceDetailContent(invoice: Invoice, payments: Vec<Payment>) -> impl IntoView {
    let order_id = get_metadata_field(&invoice, "order_id");
    let buyer_email = get_metadata_field(&invoice, "buyer_email");
    let created_display = format_date(&invoice.created_at);
    let expires_display = format_date(&invoice.expires_at);
    let confirmed_count = confirmed_payment_count(&payments);
    let payment_count = payments.len();
    let is_paid = invoice.status == types::InvoiceStatus::Paid;
    let is_expired = invoice.status == types::InvoiceStatus::Expired;

    // Customer avatar: first letter of email, or "?"
    let avatar_letter = buyer_email
        .as_ref()
        .and_then(|e| e.chars().next())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string());
    let email_display = buyer_email
        .clone()
        .unwrap_or_else(|| "No email".to_string());

    view! {
        // Title row
        <div class="invoice-detail-header">
            <div class="invoice-detail-header-left">
                <A href="/evm/invoices" attr:class="back-link">
                    <IconArrowLeft />
                    "Invoices"
                </A>
                <div class="invoice-detail-title-row">
                    <h1 class="invoice-detail-title">{invoice.id.clone()}</h1>
                    <span class=invoice.status.css_class()>{invoice.status.label()}</span>
                </div>
                {order_id.clone().map(|oid| view! {
                    <p class="invoice-detail-subtitle">"Order: "{oid}</p>
                })}
            </div>
            <div class="invoice-detail-actions">
                <button class="btn btn-secondary btn-sm">
                    <IconCopy />
                    "Copy link"
                </button>
                <button class="btn btn-secondary btn-sm">
                    <IconExport />
                    "Download"
                </button>
            </div>
        </div>

        // Content
        <div class="invoice-detail-content">
            // Main Info Cards
            <div class="invoice-detail-main">
                // Invoice Summary Card
                <div class="detail-card">
                    <div class="detail-card-header">
                        <h3>"Invoice details"</h3>
                    </div>
                    <div class="detail-card-body">
                        <div class="detail-row">
                            <span class="detail-label">"Amount due"</span>
                            <span class="detail-value detail-value-lg">
                                {format_amount(&invoice.amount, &invoice.currency)}
                            </span>
                        </div>
                        <div class="detail-row">
                            <span class="detail-label">"Amount received"</span>
                            <span class="detail-value detail-value-success">
                                {format_amount(&invoice.amount_received, &invoice.currency)}
                            </span>
                        </div>
                        <div class="detail-row">
                            <span class="detail-label">"Payments"</span>
                            <span class="detail-value">
                                {format!("{} confirmed / {} total", confirmed_count, payment_count)}
                            </span>
                        </div>
                        <div class="detail-row">
                            <span class="detail-label">"Created"</span>
                            <span class="detail-value">{created_display.clone()}</span>
                        </div>
                        <div class="detail-row">
                            <span class="detail-label">"Expires"</span>
                            <span class="detail-value">{expires_display}</span>
                        </div>
                    </div>
                </div>

                // Payments Card
                <div class="detail-card">
                    <div class="detail-card-header">
                        <h3>"Payments"</h3>
                        <span class="payment-count">{payment_count}" payment(s)"</span>
                    </div>
                    <div class="detail-card-body payments-body">
                        {if payments.is_empty() {
                            view! {
                                <div class="payments-empty">
                                    <IconClock />
                                    <span>"No payments received yet"</span>
                                </div>
                            }.into_any()
                        } else {
                            view! {
                                <div class="payments-table-container">
                                    <table class="payments-table">
                                        <thead>
                                            <tr>
                                                <th>"Transaction"</th>
                                                <th>"Asset"</th>
                                                <th>"Status"</th>
                                                <th>"From"</th>
                                            </tr>
                                        </thead>
                                        <tbody>
                                            {payments.clone().into_iter().map(|payment| {
                                                let tx_display = truncate_hex(&payment.tx_hash, 10, 8);
                                                let pstatus = payment_status(&payment);
                                                let pstatus_class = payment_status_class(&payment);
                                                let from_display = payment.from_address.as_ref()
                                                    .map(|a| truncate_hex(a, 8, 6))
                                                    .unwrap_or_else(|| "\u{2014}".to_string());

                                                view! {
                                                    <tr class="payment-row">
                                                        <td>
                                                            <div class="payment-tx-cell">
                                                                <code class="tx-hash">{tx_display}</code>
                                                                <button class="btn-icon-xs">
                                                                    <IconExternalLink />
                                                                </button>
                                                            </div>
                                                        </td>
                                                        <td>
                                                            <span class="payment-amount">{payment.asset_symbol}</span>
                                                        </td>
                                                        <td>
                                                            <span class=pstatus_class>{pstatus}</span>
                                                        </td>
                                                        <td>
                                                            <code class="tx-hash">{from_display}</code>
                                                        </td>
                                                    </tr>
                                                }
                                            }).collect_view()}
                                        </tbody>
                                    </table>
                                </div>
                            }.into_any()
                        }}
                    </div>
                </div>

                // Payment Options Card (show available payment methods)
                {(!invoice.payment_options.is_empty()).then(|| {
                    let options = invoice.payment_options.clone();
                    view! {
                        <div class="detail-card">
                            <div class="detail-card-header">
                                <h3>"Payment Options"</h3>
                            </div>
                            <div class="detail-card-body">
                                {options.into_iter().map(|opt| {
                                    let addr_display = truncate_hex(&opt.payment_address, 10, 6);
                                    view! {
                                        <div class="detail-row">
                                            <span class="detail-label">
                                                {format!("{} ({})", opt.asset_symbol, chain_name(opt.chain_id))}
                                            </span>
                                            <code class="detail-value">{addr_display}</code>
                                        </div>
                                    }
                                }).collect_view()}
                            </div>
                        </div>
                    }
                })}

                // Timeline Card
                <div class="detail-card">
                    <div class="detail-card-header">
                        <h3>"Activity"</h3>
                    </div>
                    <div class="detail-card-body">
                        <div class="timeline">
                            {is_paid.then(|| view! {
                                <div class="timeline-item timeline-item-success">
                                    <div class="timeline-dot"></div>
                                    <div class="timeline-content">
                                        <span class="timeline-title">"Invoice paid in full"</span>
                                    </div>
                                </div>
                            })}
                            {(is_expired && !is_paid).then(|| view! {
                                <div class="timeline-item timeline-item-error">
                                    <div class="timeline-dot"></div>
                                    <div class="timeline-content">
                                        <span class="timeline-title">"Invoice expired"</span>
                                    </div>
                                </div>
                            })}
                            {payments.iter().map(|p| {
                                let timestamp = format_date(&p.detected_at);
                                let desc = format!("{} payment received", p.asset_symbol);
                                view! {
                                    <div class="timeline-item">
                                        <div class="timeline-dot"></div>
                                        <div class="timeline-content">
                                            <span class="timeline-title">{desc}</span>
                                            <span class="timeline-time">{timestamp}</span>
                                        </div>
                                    </div>
                                }
                            }).collect_view()}
                            <div class="timeline-item">
                                <div class="timeline-dot"></div>
                                <div class="timeline-content">
                                    <span class="timeline-title">"Invoice created"</span>
                                    <span class="timeline-time">{created_display}</span>
                                </div>
                            </div>
                        </div>
                    </div>
                </div>
            </div>

            // Sidebar
            <div class="invoice-detail-sidebar">
                <div class="detail-card">
                    <div class="detail-card-header">
                        <h3>"Customer"</h3>
                    </div>
                    <div class="detail-card-body">
                        <div class="customer-info">
                            <div class="customer-avatar">
                                {avatar_letter}
                            </div>
                            <div class="customer-details">
                                <span class="customer-email">
                                    {email_display}
                                </span>
                            </div>
                        </div>
                    </div>
                </div>

                <div class="detail-card">
                    <div class="detail-card-header">
                        <h3>"Status Details"</h3>
                    </div>
                    <div class="detail-card-body">
                        <div class="detail-row">
                            <span class="detail-label">"Paid"</span>
                            <span class="detail-value">{if is_paid { "Yes" } else { "No" }}</span>
                        </div>
                        <div class="detail-row">
                            <span class="detail-label">"Expired"</span>
                            <span class="detail-value">{if is_expired { "Yes" } else { "No" }}</span>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::InvoiceStatus;

    #[test]
    fn test_format_date_iso() {
        assert_eq!(format_date("2024-01-15T10:30:00Z"), "Jan 15, 2024");
        assert_eq!(format_date("2024-12-25T00:00:00Z"), "Dec 25, 2024");
    }

    #[test]
    fn test_format_date_all_months() {
        let months = [
            ("01", "Jan"),
            ("02", "Feb"),
            ("03", "Mar"),
            ("04", "Apr"),
            ("05", "May"),
            ("06", "Jun"),
            ("07", "Jul"),
            ("08", "Aug"),
            ("09", "Sep"),
            ("10", "Oct"),
            ("11", "Nov"),
            ("12", "Dec"),
        ];
        for (num, name) in months {
            let input = format!("2024-{}-01T00:00:00Z", num);
            assert!(
                format_date(&input).starts_with(name),
                "Failed for month {}",
                num
            );
        }
    }

    #[test]
    fn test_format_date_short_string() {
        assert_eq!(format_date("short"), "short");
        assert_eq!(format_date(""), "");
    }

    #[test]
    fn test_chain_name() {
        assert_eq!(chain_name(1), "Ethereum");
        assert_eq!(chain_name(137), "Polygon");
        assert_eq!(chain_name(42161), "Arbitrum");
        assert_eq!(chain_name(10), "Optimism");
        assert_eq!(chain_name(8453), "Base");
        assert_eq!(chain_name(56), "BSC");
        assert_eq!(chain_name(43114), "Avalanche");
        assert_eq!(chain_name(11155111), "Sepolia");
        assert_eq!(chain_name(99999), "Unknown");
    }

    #[test]
    fn test_payment_status_confirmed() {
        let p = Payment {
            id: "p1".into(),
            chain_id: 1,
            invoice_id: "inv-1".into(),
            amount: "100".into(),
            asset_symbol: "ETH".into(),
            token_address: None,
            tx_hash: "0xabc".into(),
            block_number: Some(1),
            detected_at: "2024-01-01T00:00:00Z".into(),
            confirmed_at: Some("2024-01-01T00:05:00Z".into()),
            from_address: None,
            reorged: false,
        };
        assert_eq!(payment_status(&p), "confirmed");
        assert_eq!(payment_status_class(&p), "badge badge-success");
    }

    #[test]
    fn test_payment_status_confirming() {
        let p = Payment {
            id: "p2".into(),
            chain_id: 1,
            invoice_id: "inv-1".into(),
            amount: "100".into(),
            asset_symbol: "ETH".into(),
            token_address: None,
            tx_hash: "0xdef".into(),
            block_number: None,
            detected_at: "2024-01-01T00:00:00Z".into(),
            confirmed_at: None,
            from_address: None,
            reorged: false,
        };
        assert_eq!(payment_status(&p), "confirming");
        assert_eq!(payment_status_class(&p), "badge badge-warning");
    }

    #[test]
    fn test_payment_status_reorged() {
        let p = Payment {
            id: "p3".into(),
            chain_id: 1,
            invoice_id: "inv-1".into(),
            amount: "100".into(),
            asset_symbol: "ETH".into(),
            token_address: None,
            tx_hash: "0xghi".into(),
            block_number: Some(1),
            detected_at: "2024-01-01T00:00:00Z".into(),
            confirmed_at: Some("2024-01-01T00:05:00Z".into()),
            from_address: None,
            reorged: true,
        };
        assert_eq!(payment_status(&p), "reorged");
        assert_eq!(payment_status_class(&p), "badge badge-error");
    }

    #[test]
    fn test_get_metadata_field() {
        let invoice = Invoice {
            id: "inv-1".into(),
            currency: "USD".into(),
            status: InvoiceStatus::Pending,
            amount: "100".into(),
            amount_received: "0".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
            expires_at: "2024-01-02T00:00:00Z".into(),
            metadata: Some(serde_json::json!({"order_id": "ORD-1", "customer_email": "a@b.com"})),
            payment_options: vec![],
        };
        assert_eq!(
            get_metadata_field(&invoice, "order_id"),
            Some("ORD-1".to_string())
        );
        assert_eq!(
            get_metadata_field(&invoice, "customer_email"),
            Some("a@b.com".to_string())
        );
        assert_eq!(get_metadata_field(&invoice, "nonexistent"), None);
    }

    #[test]
    fn test_get_metadata_field_no_metadata() {
        let invoice = Invoice {
            id: "inv-2".into(),
            currency: "USD".into(),
            status: InvoiceStatus::Pending,
            amount: "50".into(),
            amount_received: "0".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
            expires_at: "2024-01-02T00:00:00Z".into(),
            metadata: None,
            payment_options: vec![],
        };
        assert_eq!(get_metadata_field(&invoice, "order_id"), None);
    }

    #[test]
    fn test_format_amount() {
        assert_eq!(format_amount("100.00", "USD"), "$100.00 USD");
        assert_eq!(format_amount("50.00", "EUR"), "\u{20ac}50.00 EUR");
        assert_eq!(format_amount("25.00", "GBP"), "\u{00a3}25.00 GBP");
        assert_eq!(format_amount("1.5", "ETH"), "1.5 ETH");
        assert_eq!(format_amount("0.001", "BTC"), "0.001 BTC");
    }

    #[test]
    fn test_confirmed_payment_count() {
        let payments = vec![
            Payment {
                id: "p1".into(),
                chain_id: 1,
                invoice_id: "inv-1".into(),
                amount: "100".into(),
                asset_symbol: "ETH".into(),
                token_address: None,
                tx_hash: "0xabc".into(),
                block_number: Some(1),
                detected_at: "2024-01-01T00:00:00Z".into(),
                confirmed_at: Some("2024-01-01T00:05:00Z".into()),
                from_address: None,
                reorged: false,
            },
            Payment {
                id: "p2".into(),
                chain_id: 1,
                invoice_id: "inv-1".into(),
                amount: "50".into(),
                asset_symbol: "ETH".into(),
                token_address: None,
                tx_hash: "0xdef".into(),
                block_number: None,
                detected_at: "2024-01-01T00:00:00Z".into(),
                confirmed_at: None,
                from_address: None,
                reorged: false,
            },
            Payment {
                id: "p3".into(),
                chain_id: 1,
                invoice_id: "inv-1".into(),
                amount: "75".into(),
                asset_symbol: "ETH".into(),
                token_address: None,
                tx_hash: "0xghi".into(),
                block_number: Some(2),
                detected_at: "2024-01-01T00:00:00Z".into(),
                confirmed_at: Some("2024-01-01T00:10:00Z".into()),
                from_address: None,
                reorged: true,
            },
        ];
        // Only p1 is confirmed and not reorged
        assert_eq!(confirmed_payment_count(&payments), 1);
        assert_eq!(confirmed_payment_count(&[]), 0);
    }

    #[test]
    fn test_truncate_hex() {
        // Long hex gets truncated
        assert_eq!(
            truncate_hex("0x1234567890abcdef1234567890abcdef", 10, 8),
            "0x12345678...90abcdef"
        );
        // Short hex stays as-is
        assert_eq!(truncate_hex("0xabc", 10, 8), "0xabc");
        // Exactly at boundary
        assert_eq!(
            truncate_hex("0x1234567890abcdef12", 10, 8),
            "0x1234567890abcdef12"
        );
    }
}

/// Clock icon for empty payments state.
#[component]
fn IconClock() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10"></circle>
            <polyline points="12 6 12 12 16 14"></polyline>
        </svg>
    }
}

/// External link icon.
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

// ============================================
// Icons
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
