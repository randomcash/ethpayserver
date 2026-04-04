//! Invoice management pages - Stripe-inspired design.
//!
//! Uses types from `crate::api::types` which mirror the backend.

use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_params_map;

use crate::api::{
    ApiError, CreateInvoiceRequest, EvmApiClient, Invoice, InvoiceStatusExt, InvoiceStatusResponse,
    Payment,
};
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

    // Create invoice modal state
    let (show_create_modal, set_show_create_modal) = signal(false);

    // Convert active filter to API status param
    let status_param = Signal::derive(move || match active_filter.get().as_str() {
        "all" => None,
        other => Some(other.to_string()),
    });

    let invoices_resource = LocalResource::new(move || {
        let api = api.get();
        let store_id = store_ctx.selected_store_id.get();
        let status = status_param.get();
        let offset = current_offset.get();
        let _ = refresh.get();

        async move {
            let Some(sid) = store_id else {
                return Err(ApiError::Network("Please select a store first".to_string()));
            };
            api.list_invoices(&sid, status.as_deref(), Some(PAGE_SIZE), Some(offset))
                .await
        }
    });

    let filters = vec![
        ("all", "All"),
        ("pending", "Pending"),
        ("processing", "Processing"),
        ("paid", "Paid"),
        ("expired", "Expired"),
    ];

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
                    <button class="btn btn-primary btn-sm" on:click=move |_| set_show_create_modal.set(true)>
                        <IconPlus />
                        "Create invoice"
                    </button>
                </div>
            </div>

            // Filters and Search
            <div class="invoices-toolbar">
                <div class="invoices-filters">
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
                        let invoices = response.invoices.clone();
                        let search = search_query.get();

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

            // Create Invoice Modal
            <CreateInvoiceModal
                show=show_create_modal
                set_show=set_show_create_modal
                on_created=move || set_refresh.update(|n| *n += 1)
            />
        </div>
    }
}

/// Create invoice modal form.
#[component]
fn CreateInvoiceModal(
    show: ReadSignal<bool>,
    set_show: WriteSignal<bool>,
    on_created: impl Fn() + 'static + Clone + Send + Sync,
) -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");
    let store_ctx = use_context::<StoreContext>().expect("StoreContext must be provided");

    let (amount, set_amount) = signal(String::new());
    let (currency, set_currency) = signal("USD".to_string());
    let (expiration_minutes, set_expiration_minutes) = signal("15".to_string());
    let (error, set_error) = signal(Option::<String>::None);
    let (submitting, set_submitting) = signal(false);

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let api = api.get();
        let store_id = store_ctx.selected_store_id.get();
        let amount_val = amount.get();
        let currency_val = currency.get();
        let exp_min = expiration_minutes.get();
        let on_created = on_created.clone();

        set_error.set(None);
        set_submitting.set(true);

        leptos::task::spawn_local(async move {
            let Some(store_id) = store_id else {
                set_error.set(Some("Please select a store first".to_string()));
                set_submitting.set(false);
                return;
            };

            if amount_val.is_empty() {
                set_error.set(Some("Amount is required".to_string()));
                set_submitting.set(false);
                return;
            }

            let exp_seconds = exp_min.parse::<u64>().ok().map(|m| m * 60);

            let request = CreateInvoiceRequest {
                store_id,
                currency: currency_val,
                amount: amount_val,
                expiration_seconds: exp_seconds,
                metadata: None,
                webhook_url: None,
                redirect_url: None,
            };

            match api.create_invoice(&request).await {
                Ok(_invoice) => {
                    set_show.set(false);
                    set_amount.set(String::new());
                    set_currency.set("USD".to_string());
                    set_expiration_minutes.set("15".to_string());
                    on_created();
                }
                Err(e) => {
                    set_error.set(Some(e.to_string()));
                }
            }
            set_submitting.set(false);
        });
    };

    move || {
        if !show.get() {
            return None;
        }
        Some(view! {
            <div class="modal-overlay" on:click=move |_| set_show.set(false)>
                <div class="modal" on:click=|ev| ev.stop_propagation()>
                    <div class="modal-header">
                        <h2>"Create Invoice"</h2>
                        <button class="btn btn-ghost btn-sm btn-icon" on:click=move |_| set_show.set(false)>
                            <IconClose />
                        </button>
                    </div>
                    <form on:submit=on_submit.clone()>
                        <div class="modal-body">
                            {move || error.get().map(|e| view! {
                                <div class="form-error">{e}</div>
                            })}

                            <div class="form-group">
                                <label for="amount">"Amount"</label>
                                <input
                                    type="text"
                                    id="amount"
                                    class="form-input"
                                    placeholder="100.00"
                                    prop:value=move || amount.get()
                                    on:input=move |ev| set_amount.set(event_target_value(&ev))
                                    required
                                />
                            </div>

                            <div class="form-group">
                                <label for="currency">"Currency"</label>
                                <select
                                    id="currency"
                                    class="form-input"
                                    prop:value=move || currency.get()
                                    on:change=move |ev| set_currency.set(event_target_value(&ev))
                                >
                                    <option value="USD">"USD"</option>
                                    <option value="EUR">"EUR"</option>
                                    <option value="ETH">"ETH"</option>
                                    <option value="BTC">"BTC"</option>
                                </select>
                            </div>

                            <div class="form-group">
                                <label for="expiration">"Expiration (minutes)"</label>
                                <input
                                    type="number"
                                    id="expiration"
                                    class="form-input"
                                    placeholder="15"
                                    prop:value=move || expiration_minutes.get()
                                    on:input=move |ev| set_expiration_minutes.set(event_target_value(&ev))
                                />
                            </div>
                        </div>
                        <div class="modal-footer">
                            <button type="button" class="btn btn-secondary btn-sm" on:click=move |_| set_show.set(false)>
                                "Cancel"
                            </button>
                            <button type="submit" class="btn btn-primary btn-sm" disabled=move || submitting.get()>
                                {move || if submitting.get() { "Creating..." } else { "Create Invoice" }}
                            </button>
                        </div>
                    </form>
                </div>
            </div>
        })
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

/// Invoice detail page - fetches from GET /invoices/{id}/status.
#[component]
pub fn InvoiceDetailPage() -> impl IntoView {
    let params = use_params_map();
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");

    let invoice_id = Signal::derive(move || params.get().get("id").unwrap_or_default());

    let status_resource = LocalResource::new(move || {
        let api = api.get();
        let id = invoice_id.get();
        async move {
            if id.is_empty() {
                return Err(ApiError::Network("No invoice ID".to_string()));
            }
            api.get_invoice_status(&id).await
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
                {move || status_resource.get().map(|result| match &*result {
                    Err(e) => view! {
                        <div class="error-container">
                            <p class="error-message">{e.to_string()}</p>
                        </div>
                    }.into_any(),
                    Ok(status) => {
                        view! { <InvoiceDetailContent status=status.clone() /> }.into_any()
                    }
                })}
            </Suspense>
        </div>
    }
}

/// Inner content of the invoice detail page (rendered after data loads).
#[component]
fn InvoiceDetailContent(status: InvoiceStatusResponse) -> impl IntoView {
    let order_id = status.payments.first().and(None::<String>); // metadata not on status response
    let created_display = "—".to_string(); // created_at not on status response
    let expires_display = format_date(&status.expires_at);
    let payment_count = status.payment_count;
    let payments = status.payments.clone();

    view! {
        // Title row
        <div class="invoice-detail-header">
            <div class="invoice-detail-header-left">
                <A href="/evm/invoices" attr:class="back-link">
                    <IconArrowLeft />
                    "Invoices"
                </A>
                <div class="invoice-detail-title-row">
                    <h1 class="invoice-detail-title">{status.id.clone()}</h1>
                    <span class=status.status.css_class()>{status.status.label()}</span>
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
                                {format!("{} {}", status.amount, status.currency)}
                            </span>
                        </div>
                        <div class="detail-row">
                            <span class="detail-label">"Amount received"</span>
                            <span class="detail-value detail-value-success">
                                {format!("{} {}", status.amount_received, status.currency)}
                            </span>
                        </div>
                        <div class="detail-row">
                            <span class="detail-label">"Payments"</span>
                            <span class="detail-value">
                                {format!("{} confirmed / {} total", status.confirmed_count, status.payment_count)}
                            </span>
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
                                                let tx_display = if payment.tx_hash.len() > 18 {
                                                    format!("{}...{}",
                                                        &payment.tx_hash[..10],
                                                        &payment.tx_hash[payment.tx_hash.len()-8..])
                                                } else {
                                                    payment.tx_hash.clone()
                                                };
                                                let pstatus = payment_status(&payment);
                                                let pstatus_class = payment_status_class(&payment);
                                                let from_display = payment.from_address.as_ref()
                                                    .map(|a| if a.len() > 18 {
                                                        format!("{}...{}", &a[..8], &a[a.len()-6..])
                                                    } else { a.clone() })
                                                    .unwrap_or_else(|| "—".to_string());

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
                {(!status.payment_options.is_empty()).then(|| {
                    let options = status.payment_options.clone();
                    view! {
                        <div class="detail-card">
                            <div class="detail-card-header">
                                <h3>"Payment Options"</h3>
                            </div>
                            <div class="detail-card-body">
                                {options.into_iter().map(|opt| {
                                    let addr_display = if opt.payment_address.len() > 18 {
                                        format!("{}...{}", &opt.payment_address[..10], &opt.payment_address[opt.payment_address.len()-6..])
                                    } else {
                                        opt.payment_address.clone()
                                    };
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
                            {(status.is_paid).then(|| view! {
                                <div class="timeline-item timeline-item-success">
                                    <div class="timeline-dot"></div>
                                    <div class="timeline-content">
                                        <span class="timeline-title">"Invoice paid in full"</span>
                                    </div>
                                </div>
                            })}
                            {(status.is_expired && !status.is_paid).then(|| view! {
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
                                {"?"}
                            </div>
                            <div class="customer-details">
                                <span class="customer-email">
                                    {"No email"}
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
                            <span class="detail-value">{if status.is_paid { "Yes" } else { "No" }}</span>
                        </div>
                        <div class="detail-row">
                            <span class="detail-label">"Expired"</span>
                            <span class="detail-value">{if status.is_expired { "Yes" } else { "No" }}</span>
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
fn IconClose() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="18" y1="6" x2="6" y2="18"></line>
            <line x1="6" y1="6" x2="18" y2="18"></line>
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
