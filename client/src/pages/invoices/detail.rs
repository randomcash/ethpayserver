//! Invoice detail page and its inner content component.

use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_params_map;

use crate::api::{ApiError, EvmApiClient, Invoice, InvoiceStatusExt, Payment};

use super::helpers::{
    IconExport, chain_name, confirmed_payment_count, format_amount, format_date,
    get_metadata_field, payment_status, payment_status_class, truncate_hex,
};

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

/// Left arrow icon for the back link.
#[component]
fn IconArrowLeft() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="19" y1="12" x2="5" y2="12"></line>
            <polyline points="12 19 5 12 12 5"></polyline>
        </svg>
    }
}

/// Copy icon for the "Copy link" action.
#[component]
fn IconCopy() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect>
            <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>
        </svg>
    }
}
