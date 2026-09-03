//! Payments list page and its row/card sub-components.

use leptos::prelude::*;
use leptos_router::components::A;

use crate::api::{ApiError, EvmApiClient, Payment};
use crate::app::{StoreContext, StoresStatus};
use crate::components::{NoStoreSelected, PAGE_SIZE, Pagination};
use crate::services::StatusUpdate;
use crate::util::{chain_name, short_store_id};

use super::format::{
    format_crypto_amount, format_date, payment_status, payment_status_class, truncate_hash,
};
use super::icons::{IconChevronRight, IconExport, IconExternalLink, IconMore, IconSearch};

/// Payments list page.
#[component]
pub fn PaymentsPage() -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");
    let store_ctx = use_context::<StoreContext>().expect("StoreContext must be provided");

    // Inputs for the no-store-selected branch below: whether the store fetch
    // has landed, whether it found anything, and a way to retry a failed one.
    let store_status = store_ctx.stores_status;
    let has_stores = Signal::derive({
        let ctx = store_ctx.clone();
        move || !ctx.stores.get().is_empty()
    });
    let retry_stores = Callback::new({
        let ctx = store_ctx.clone();
        move |()| ctx.refetch_stores()
    });

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

    // WebSocket-driven payment patches and new-payment detection.
    // Stores payment_id -> new status for in-place patching.
    // If a PaymentUpdate arrives for an unknown payment_id, triggers a refetch.
    let (ws_patches, set_ws_patches) = signal(std::collections::HashMap::<String, String>::new());
    let ws_update = use_context::<ReadSignal<Option<StatusUpdate>>>();
    let known_payment_ids: StoredValue<std::collections::HashSet<String>> =
        StoredValue::new(std::collections::HashSet::new());
    if let Some(ws_update) = ws_update {
        Effect::new(move || {
            if let Some(StatusUpdate::PaymentUpdate {
                ref payment_id,
                status: ref new_status,
                ..
            }) = ws_update.get()
            {
                let is_known = known_payment_ids.with_value(|ids| ids.contains(payment_id));
                if is_known {
                    set_ws_patches.update(|patches| {
                        patches.insert(payment_id.clone(), new_status.clone());
                    });
                } else {
                    // New payment — trigger full refetch.
                    set_refresh.update(|n| *n = n.wrapping_add(1));
                }
            }
        });
    }
    // Clear patches on fresh fetch.
    Effect::new(move || {
        let _ = refresh.get();
        set_ws_patches.update(|patches| patches.clear());
    });

    // Convert active filter to API status param
    let status_param = Signal::derive(move || match active_filter.get().as_str() {
        "all" => None,
        other => Some(other.to_string()),
    });

    let payments_resource = LocalResource::new(move || {
        let api = api.get();
        let store_id = store_ctx.selected_store_id.get();
        let stores_loaded = matches!(store_status.get(), StoresStatus::Loaded);
        let status = status_param.get();
        let offset = current_offset.get();
        let _ = refresh.get();

        async move {
            // Mirrors `pages/invoices/list.rs` — see the reasoning there
            // (RCS-171): "All Stores" is a real query, but only once the store
            // list has landed, and a non-admin's 400 is a "pick a store" state
            // rather than an error to render raw.
            if store_id.is_none() && !stores_loaded {
                return Ok(None);
            }
            match api
                .list_payments(
                    store_id.as_deref(),
                    status.as_deref(),
                    Some(PAGE_SIZE),
                    Some(offset),
                )
                .await
            {
                Ok(response) => Ok(Some(response)),
                Err(ApiError::Http { status: 400, .. }) if store_id.is_none() => Ok(None),
                Err(e) => Err(e),
            }
        }
    });

    // Only worth a column when rows can come from different stores.
    let show_store = Signal::derive(move || store_ctx.selected_store_id.get().is_none());

    let filters = vec![
        ("all", "All"),
        ("pending", "Pending"),
        ("confirmed", "Confirmed"),
    ];

    // The CSV export hits the same server gate as the list: with no store
    // selected it is admin-only, and a non-admin's request comes back 400. The
    // list already resolves to `None` in exactly that case - that is what puts
    // NoStoreSelected on screen - so the same signal says whether an export can
    // succeed. Offering the button there fired a request that could only fail,
    // and the failure was console-only, so to the user the button did nothing
    // at all. RCS-171 asks for non-admin behaviour to be handled gracefully
    // with no raw error; a button that silently does nothing is not that.
    let export_available = move || matches!(payments_resource.get().as_deref(), Some(Ok(Some(_))));

    view! {
        <div class="payments-page">
            // Page Header
            <div class="page-header-row">
                <div>
                    <h1 class="page-title">"Payments"</h1>
                    <p class="page-description">"View all received payments across networks"</p>
                </div>
                <div class="page-actions">
                    <button
                        class="btn btn-secondary btn-sm"
                        disabled=move || !export_available()
                        on:click=move |_| {
                        let api = api.get();
                        let store_id = store_ctx.selected_store_id.get();
                        let status = status_param.get();
                        wasm_bindgen_futures::spawn_local(async move {
                            match api.export_payments_csv(store_id.as_deref(), status.as_deref()).await {
                                Ok(csv) => crate::pages::trigger_csv_download(&csv, "payments.csv"),
                                Err(e) => {
                                    web_sys::console::error_1(
                                        &format!("CSV export failed: {e}").into(),
                                    );
                                }
                            }
                        });
                    }>
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
                    // Not an inline empty state: "no store selected" also covers
                    // stores still loading, a failed fetch, and a deliberate
                    // "All Stores" choice. NoStoreSelected tells them apart.
                    Ok(None) => view! {
                        <NoStoreSelected
                            entity="Payments"
                            status=store_status
                            has_stores=has_stores
                            on_retry=retry_stores
                        />
                    }
                    .into_any(),
                    Ok(Some(response)) => {
                        let total = response.total;
                        let mut payments = response.payments.clone();
                        let search = search_query.get();

                        // Track known payment IDs so WS can distinguish new vs existing.
                        known_payment_ids.set_value(
                            payments.iter().map(|p| p.id.clone()).collect(),
                        );

                        // Apply WebSocket status patches in-place.
                        let patches = ws_patches.get();
                        if !patches.is_empty() {
                            for payment in &mut payments {
                                if let Some(new_status) = patches.get(&payment.id) {
                                    match new_status.as_str() {
                                        "confirmed" => {
                                            if payment.confirmed_at.is_none() {
                                                payment.confirmed_at =
                                                    Some(String::new());
                                            }
                                            payment.reorged = false;
                                        }
                                        "reorged" => {
                                            payment.reorged = true;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }

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

                            view! {
                                // Payment Table (Desktop)
                                <div class="payments-table-container desktop-only">
                                    <table class="payments-table">
                                        <thead>
                                            <tr>
                                                <th>"Transaction"</th>
                                                {show_store.get().then(|| view! { <th>"Store"</th> })}
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
                                                view! { <PaymentRow payment=payment show_store=show_store.get() /> }
                                            }).collect_view()}
                                        </tbody>
                                    </table>
                                </div>

                                // Payment Cards (Mobile)
                                <div class="payments-cards mobile-only">
                                    {filtered.into_iter().map(|payment| {
                                        view! { <PaymentCard payment=payment show_store=show_store.get() /> }
                                    }).collect_view()}
                                </div>

                                // Pagination
                                <Pagination
                                    total=total
                                    page_size=PAGE_SIZE
                                    current_offset=offset
                                    on_page_change=move |new_offset| set_current_offset.set(new_offset)
                                    item_label="payments"
                                />
                            }.into_any()
                        }
                    }
                })}
            </Suspense>
        </div>
    }
}

/// Label for the store a payment belongs to.
///
/// Falls back to a shortened store ID when the server could not resolve a name,
/// so the "All Stores" view never leaves a row unattributed.
fn store_label(payment: &Payment) -> String {
    match payment.store_name.as_deref() {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => short_store_id(payment.store_id.as_deref().unwrap_or_default()),
    }
}

/// Payment table row.
///
/// `show_store` adds the store column, which the list page turns on only for
/// the "All Stores" view — see above (RCS-171).
#[component]
fn PaymentRow(payment: Payment, show_store: bool) -> impl IntoView {
    let store_display = show_store.then(|| store_label(&payment));
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
        format_crypto_amount(&payment.amount, payment.decimals),
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
            {store_display.map(|name| view! {
                <td>
                    <span class="payment-store">{name}</span>
                </td>
            })}
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
///
/// `show_store` behaves as on [`PaymentRow`].
#[component]
fn PaymentCard(payment: Payment, show_store: bool) -> impl IntoView {
    let store_display = show_store.then(|| store_label(&payment));
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
        format_crypto_amount(&payment.amount, payment.decimals),
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
                {store_display.map(|name| view! {
                    <span class="payment-card-store">{name}</span>
                })}
                <A href=format!("/evm/payments/{}", payment_link) attr:class="btn btn-ghost btn-xs">
                    "Details"
                    <IconChevronRight />
                </A>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::store_label;
    use crate::api::Payment;

    fn payment(store_id: Option<&str>, store_name: Option<&str>) -> Payment {
        Payment {
            id: "pay-1".to_string(),
            store_id: store_id.map(str::to_string),
            store_name: store_name.map(str::to_string),
            chain_id: 1,
            invoice_id: "inv-1".to_string(),
            tx_hash: "0xabc".to_string(),
            amount: "1".to_string(),
            asset_symbol: "ETH".to_string(),
            token_address: None,
            block_number: None,
            from_address: None,
            detected_at: "2024-01-01T00:00:00Z".to_string(),
            confirmed_at: None,
            reorged: false,
            decimals: 18,
        }
    }

    #[test]
    fn prefers_the_store_name() {
        assert_eq!(
            store_label(&payment(
                Some("11111111-2222-3333-4444-555555555555"),
                Some("Acme")
            )),
            "Acme"
        );
    }

    #[test]
    fn falls_back_to_a_shortened_id() {
        assert_eq!(
            store_label(&payment(Some("11111111-2222-3333-4444-555555555555"), None)),
            "11111111\u{2026}"
        );
        assert_eq!(store_label(&payment(Some("short"), None)), "short");
    }

    #[test]
    fn unattributed_payments_still_get_a_label() {
        // `store_id` is None on every non-list endpoint, and on a list row whose
        // invoice could not be read. Neither should render a blank cell.
        assert_eq!(store_label(&payment(None, None)), "Unknown store");
        assert_eq!(store_label(&payment(Some(""), Some(""))), "Unknown store");
    }
}
