//! Row/card components and icons used by the invoice list page.

use leptos::prelude::*;
use leptos_router::components::A;

use crate::api::{Invoice, InvoiceStatusExt};

use super::helpers::{format_amount, format_date, get_metadata_field};

/// Invoice table row.
#[component]
pub(super) fn InvoiceRow(invoice: Invoice) -> impl IntoView {
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
pub(super) fn InvoiceCard(invoice: Invoice) -> impl IntoView {
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

/// Overflow ("more") icon for the row actions.
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

/// Chevron right icon for mobile cards.
#[component]
fn IconChevronRight() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="9 18 15 12 9 6"></polyline>
        </svg>
    }
}

/// Plus icon for the "Create invoice" button.
#[component]
pub(super) fn IconPlus() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="12" y1="5" x2="12" y2="19"></line>
            <line x1="5" y1="12" x2="19" y2="12"></line>
        </svg>
    }
}

/// Search icon for the filter inputs.
#[component]
pub(super) fn IconSearch() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="11" cy="11" r="8"></circle>
            <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
        </svg>
    }
}
