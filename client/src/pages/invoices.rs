//! Invoice management pages - Stripe-inspired design.
//!
//! Uses types from `crate::api::types` which mirror the backend.

use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_params_map;

use crate::api::{AssetType, Invoice, InvoiceStatus, Payment};

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

/// Invoice list page.
#[component]
pub fn InvoicesPage() -> impl IntoView {
    // Filter state
    let (active_filter, set_active_filter) = signal("all".to_string());
    let (search_query, set_search_query) = signal(String::new());

    // Mock invoices - will come from API
    // Note: Invoice uses types from crate::api::types which mirror the backend
    let invoices: Vec<Invoice> = vec![
        Invoice {
            id: "INV-0001".to_string(),
            store_id: "store-1".to_string(),
            currency: "USD".to_string(),
            status: InvoiceStatus::Paid,
            amount: "150.00".to_string(),
            amount_received: "150.00".to_string(),
            created_at: "2024-01-15T10:30:00Z".to_string(),
            expires_at: "2024-01-16T10:30:00Z".to_string(),
            metadata: Some(serde_json::json!({"order_id": "ORD-1234", "customer_email": "alice@example.com"})),
        },
        Invoice {
            id: "INV-0002".to_string(),
            store_id: "store-1".to_string(),
            currency: "USD".to_string(),
            status: InvoiceStatus::Pending,
            amount: "89.99".to_string(),
            amount_received: "0.00".to_string(),
            created_at: "2024-01-14T09:00:00Z".to_string(),
            expires_at: "2024-01-15T09:00:00Z".to_string(),
            metadata: Some(serde_json::json!({"order_id": "ORD-1235", "customer_email": "bob@example.com"})),
        },
        Invoice {
            id: "INV-0003".to_string(),
            store_id: "store-1".to_string(),
            currency: "USD".to_string(),
            status: InvoiceStatus::Paid,
            amount: "500.00".to_string(),
            amount_received: "500.00".to_string(),
            created_at: "2024-01-13T14:00:00Z".to_string(),
            expires_at: "2024-01-14T14:00:00Z".to_string(),
            metadata: Some(serde_json::json!({"customer_email": "charlie@example.com"})),
        },
        Invoice {
            id: "INV-0004".to_string(),
            store_id: "store-1".to_string(),
            currency: "USD".to_string(),
            status: InvoiceStatus::Expired,
            amount: "75.00".to_string(),
            amount_received: "0.00".to_string(),
            created_at: "2024-01-12T08:00:00Z".to_string(),
            expires_at: "2024-01-12T20:00:00Z".to_string(),
            metadata: Some(serde_json::json!({"order_id": "ORD-1236"})),
        },
        Invoice {
            id: "INV-0005".to_string(),
            store_id: "store-1".to_string(),
            currency: "USD".to_string(),
            status: InvoiceStatus::Processing,
            amount: "1200.00".to_string(),
            amount_received: "800.00".to_string(),
            created_at: "2024-01-11T16:00:00Z".to_string(),
            expires_at: "2024-01-12T16:00:00Z".to_string(),
            metadata: Some(serde_json::json!({"order_id": "ORD-1237", "customer_email": "diana@example.com"})),
        },
        Invoice {
            id: "INV-0006".to_string(),
            store_id: "store-1".to_string(),
            currency: "USD".to_string(),
            status: InvoiceStatus::PartiallyPaid,
            amount: "300.00".to_string(),
            amount_received: "150.00".to_string(),
            created_at: "2024-01-10T12:00:00Z".to_string(),
            expires_at: "2024-01-11T12:00:00Z".to_string(),
            metadata: Some(serde_json::json!({"order_id": "ORD-1238"})),
        },
    ];

    let filters = vec![
        ("all", "All", None),
        ("pending", "Pending", Some(2)),
        ("processing", "Processing", Some(1)),
        ("paid", "Paid", Some(2)),
        ("expired", "Expired", Some(1)),
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
                    <button class="btn btn-primary btn-sm">
                        <IconPlus />
                        "Create invoice"
                    </button>
                </div>
            </div>

            // Filters and Search
            <div class="invoices-toolbar">
                <div class="invoices-filters">
                    {filters.into_iter().map(|(key, label, count)| {
                        let key_owned = key.to_string();
                        let key_for_click = key.to_string();
                        view! {
                            <button
                                class=move || if active_filter.get() == key_owned { "filter-tab active" } else { "filter-tab" }
                                on:click=move |_| set_active_filter.set(key_for_click.clone())
                            >
                                {label}
                                {count.map(|c| view! { <span class="filter-count">{c}</span> })}
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
                        {invoices.clone().into_iter().map(|invoice| {
                            view! { <InvoiceRow invoice=invoice /> }
                        }).collect_view()}
                    </tbody>
                </table>
            </div>

            // Invoice Cards (Mobile)
            <div class="invoices-cards mobile-only">
                {invoices.into_iter().map(|invoice| {
                    view! { <InvoiceCard invoice=invoice /> }
                }).collect_view()}
            </div>

            // Pagination
            <div class="invoices-pagination">
                <div class="pagination-info">
                    "Showing "<strong>"1-5"</strong>" of "<strong>"24"</strong>" invoices"
                </div>
                <div class="pagination-controls">
                    <button class="btn btn-ghost btn-sm" disabled>"Previous"</button>
                    <button class="btn btn-ghost btn-sm">"Next"</button>
                </div>
            </div>
        </div>
    }
}

/// Helper to extract a field from invoice metadata.
fn get_metadata_field(invoice: &Invoice, field: &str) -> Option<String> {
    invoice.metadata.as_ref()
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
                "01" => "Jan", "02" => "Feb", "03" => "Mar", "04" => "Apr",
                "05" => "May", "06" => "Jun", "07" => "Jul", "08" => "Aug",
                "09" => "Sep", "10" => "Oct", "11" => "Nov", "12" => "Dec",
                _ => parts[1],
            };
            return format!("{} {}, {}", month, parts[2], parts[0]);
        }
    }
    iso.to_string()
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
                    <span class="invoice-fiat">{format!("${} {}", invoice.amount, invoice.currency)}</span>
                </div>
            </td>
            <td>
                <div class="invoice-amount-cell">
                    <span class="invoice-received">{format!("${}", invoice.amount_received)}</span>
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
                    <span class="invoice-card-value">{format!("${} {}", invoice.amount, invoice.currency)}</span>
                </div>
                <div class="invoice-card-amount">
                    <span class="invoice-card-label">"Received"</span>
                    <span class="invoice-card-value">{format!("${}", invoice.amount_received)}</span>
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

/// Invoice detail page.
#[component]
pub fn InvoiceDetailPage() -> impl IntoView {
    let params = use_params_map();
    let invoice_id = move || params.get().get("id").unwrap_or_default();

    // Mock invoice - will come from API
    let invoice = Invoice {
        id: invoice_id(),
        store_id: "store-1".to_string(),
        currency: "USD".to_string(),
        status: InvoiceStatus::Paid,
        amount: "500.00".to_string(),
        amount_received: "500.00".to_string(),
        created_at: "2024-01-15T10:30:00Z".to_string(),
        expires_at: "2024-01-16T10:30:00Z".to_string(),
        metadata: Some(serde_json::json!({
            "order_id": "ORD-1234",
            "customer_email": "alice@example.com"
        })),
    };

    // Mock payments for this invoice - fetched separately from API
    let payments: Vec<Payment> = vec![
        Payment {
            id: "pay-001".to_string(),
            invoice_id: invoice.id.clone(),
            chain_id: 1,
            asset_type: AssetType::Native,
            amount: "140000000000000000".to_string(), // 0.14 ETH in wei
            asset_symbol: "ETH".to_string(),
            token_address: None,
            tx_hash: "0x1a2b3c4d5e6f7890abcdef1234567890abcdef1234567890abcdef1234567890".to_string(),
            block_number: Some(19234567),
            detected_at: "2024-01-15T10:45:00Z".to_string(),
            confirmed_at: Some("2024-01-15T10:50:00Z".to_string()),
            from_address: Some("0x1234567890abcdef1234567890abcdef12345678".to_string()),
            reorged: false,
            credited_amount: Some("250.00".to_string()),
            rate_used: Some("0.00056".to_string()),
        },
        Payment {
            id: "pay-002".to_string(),
            invoice_id: invoice.id.clone(),
            chain_id: 1,
            asset_type: AssetType::ERC20,
            amount: "250000000".to_string(), // 250 USDC (6 decimals)
            asset_symbol: "USDC".to_string(),
            token_address: Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string()),
            tx_hash: "0x9876543210fedcba9876543210fedcba9876543210fedcba9876543210fedcba".to_string(),
            block_number: Some(19234612),
            detected_at: "2024-01-15T11:15:00Z".to_string(),
            confirmed_at: Some("2024-01-15T11:20:00Z".to_string()),
            from_address: Some("0xabcdef1234567890abcdef1234567890abcdef12".to_string()),
            reorged: false,
            credited_amount: Some("250.00".to_string()),
            rate_used: Some("1.0".to_string()),
        },
    ];

    let order_id = get_metadata_field(&invoice, "order_id");
    let customer_email = get_metadata_field(&invoice, "customer_email");
    let created_display = format_date(&invoice.created_at);
    let expires_display = format_date(&invoice.expires_at);
    let payment_count = payments.len();

    view! {
        <div class="invoice-detail-page">
            // Header
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
                                    {format!("${} {}", invoice.amount, invoice.currency)}
                                </span>
                            </div>
                            <div class="detail-row">
                                <span class="detail-label">"Amount received"</span>
                                <span class="detail-value detail-value-success">
                                    {format!("${} {}", invoice.amount_received, invoice.currency)}
                                </span>
                            </div>
                            <div class="detail-row">
                                <span class="detail-label">"Created"</span>
                                <span class="detail-value">{created_display}</span>
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
                                                    <th>"Amount"</th>
                                                    <th>"Network"</th>
                                                    <th>"Status"</th>
                                                    <th>"Credited"</th>
                                                </tr>
                                            </thead>
                                            <tbody>
                                                {payments.clone().into_iter().map(|payment| {
                                                    let tx_display = format!("{}...{}",
                                                        &payment.tx_hash[..10],
                                                        &payment.tx_hash[payment.tx_hash.len()-8..]);
                                                    let network = chain_name(payment.chain_id);
                                                    let status = payment_status(&payment);
                                                    let status_class = payment_status_class(&payment);
                                                    let credited = payment.credited_amount
                                                        .map(|c| format!("${}", c))
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
                                                                <span class="payment-network">{network}</span>
                                                            </td>
                                                            <td>
                                                                <span class=status_class>{status}</span>
                                                            </td>
                                                            <td>
                                                                <span class="payment-credited">{credited}</span>
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

                    // Timeline Card
                    <div class="detail-card">
                        <div class="detail-card-header">
                            <h3>"Activity"</h3>
                        </div>
                        <div class="detail-card-body">
                            <div class="timeline">
                                {(invoice.status == InvoiceStatus::Paid).then(|| view! {
                                    <div class="timeline-item timeline-item-success">
                                        <div class="timeline-dot"></div>
                                        <div class="timeline-content">
                                            <span class="timeline-title">"Invoice paid in full"</span>
                                            <span class="timeline-time">"Jan 15, 2024 at 11:20 AM"</span>
                                        </div>
                                    </div>
                                })}
                                {payments.iter().map(|p| {
                                    let timestamp = format_date(&p.detected_at);
                                    let amount = format!("{} on {}", p.asset_symbol, chain_name(p.chain_id));
                                    view! {
                                        <div class="timeline-item">
                                            <div class="timeline-dot"></div>
                                            <div class="timeline-content">
                                                <span class="timeline-title">"Payment received: "{amount}</span>
                                                <span class="timeline-time">{timestamp}</span>
                                            </div>
                                        </div>
                                    }
                                }).collect_view()}
                                <div class="timeline-item">
                                    <div class="timeline-dot"></div>
                                    <div class="timeline-content">
                                        <span class="timeline-title">"Invoice created"</span>
                                        <span class="timeline-time">{format_date(&invoice.created_at)}</span>
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
                                    {customer_email.clone()
                                        .map(|e| e.chars().next().unwrap_or('?').to_uppercase().to_string())
                                        .unwrap_or("?".to_string())}
                                </div>
                                <div class="customer-details">
                                    <span class="customer-email">
                                        {customer_email.unwrap_or_else(|| "No email".to_string())}
                                    </span>
                                </div>
                            </div>
                        </div>
                    </div>

                    <div class="detail-card">
                        <div class="detail-card-header">
                            <h3>"Metadata"</h3>
                        </div>
                        <div class="detail-card-body">
                            {invoice.metadata.as_ref().map(|m| {
                                view! {
                                    <pre class="metadata-json">{serde_json::to_string_pretty(m).unwrap_or_default()}</pre>
                                }.into_any()
                            }).unwrap_or_else(|| view! {
                                <div class="metadata-empty">
                                    <span>"No metadata"</span>
                                </div>
                            }.into_any())}
                        </div>
                    </div>
                </div>
            </div>
        </div>
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
