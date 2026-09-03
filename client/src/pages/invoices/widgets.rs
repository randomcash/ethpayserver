//! Row/card components and icons used by the invoice list page.

use leptos::prelude::*;
use leptos_router::components::A;

use crate::api::{Invoice, InvoiceStatusExt};

use super::helpers::{format_amount, format_date, get_metadata_field};
use crate::util::short_store_id;

/// Label for the store an invoice belongs to.
///
/// Falls back to a shortened store ID when the server could not resolve a name,
/// so the "All Stores" view never shows a row with no attribution at all.
fn store_label(invoice: &Invoice) -> String {
    match invoice.store_name.as_deref() {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => short_store_id(&invoice.store_id),
    }
}

/// Invoice table row.
///
/// `show_store` adds the store column, which the list page turns on only for
/// the "All Stores" view — see `list.rs` (RCS-171).
#[component]
pub(super) fn InvoiceRow(invoice: Invoice, show_store: bool) -> impl IntoView {
    let invoice_id = invoice.id.clone();
    let invoice_link = invoice.id.clone();
    let order_id = get_metadata_field(&invoice, "order_id");
    let created_display = format_date(&invoice.created_at);
    let store_display = show_store.then(|| store_label(&invoice));

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
            {store_display.map(|name| view! {
                <td>
                    <span class="invoice-store">{name}</span>
                </td>
            })}
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
///
/// `show_store` behaves as on [`InvoiceRow`].
#[component]
pub(super) fn InvoiceCard(invoice: Invoice, show_store: bool) -> impl IntoView {
    let invoice_id = invoice.id.clone();
    let invoice_link = invoice.id.clone();
    let order_id = get_metadata_field(&invoice, "order_id");
    let created_display = format_date(&invoice.created_at);
    let store_display = show_store.then(|| store_label(&invoice));

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
                {store_display.map(|name| view! {
                    <span class="invoice-card-store">{name}</span>
                })}
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

#[cfg(test)]
mod tests {
    use super::store_label;
    use crate::api::{Invoice, InvoiceStatus};

    fn invoice(store_id: &str, store_name: Option<&str>) -> Invoice {
        Invoice {
            id: "inv-1".to_string(),
            store_id: store_id.to_string(),
            store_name: store_name.map(str::to_string),
            currency: "USD".to_string(),
            status: InvoiceStatus::Pending,
            amount: "10.00".to_string(),
            amount_received: "0.00".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            expires_at: "2024-01-02T00:00:00Z".to_string(),
            metadata: None,
            payment_options: vec![],
        }
    }

    #[test]
    fn prefers_the_store_name() {
        assert_eq!(
            store_label(&invoice(
                "11111111-2222-3333-4444-555555555555",
                Some("Acme")
            )),
            "Acme"
        );
    }

    #[test]
    fn falls_back_to_a_shortened_id() {
        // The server could not resolve a name — the row still has to say which
        // store it came from, so show the ID rather than nothing.
        assert_eq!(
            store_label(&invoice("11111111-2222-3333-4444-555555555555", None)),
            "11111111\u{2026}"
        );
        assert_eq!(store_label(&invoice("short", None)), "short");
    }

    #[test]
    fn empty_name_is_treated_as_missing() {
        assert_eq!(store_label(&invoice("short", Some(""))), "short");
    }

    #[test]
    fn falls_back_again_when_there_is_no_id_either() {
        assert_eq!(store_label(&invoice("", None)), "Unknown store");
    }
}
