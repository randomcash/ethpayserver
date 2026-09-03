//! Invoice list page (table + cards) and its row components.

use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use crate::api::{ApiError, EvmApiClient, Invoice};
use crate::app::{StoreContext, StoresStatus};
use crate::components::{CreateInvoiceSignal, NoStoreSelected, PAGE_SIZE, Pagination};
use crate::services::StatusUpdate;

use super::helpers::IconExport;
use super::widgets::{IconPlus, IconSearch, InvoiceCard, InvoiceRow};

/// Invoice list page.
#[component]
pub fn InvoicesPage() -> impl IntoView {
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
    let (currency_filter, set_currency_filter) = signal("all".to_string());
    let (search_query, set_search_query) = signal(String::new());
    let (current_offset, set_current_offset) = signal(0i64);

    // TX hash lookup state
    let (tx_search_input, set_tx_search_input) = signal(String::new());
    let (tx_search_error, set_tx_search_error) = signal(Option::<String>::None);
    let (tx_searching, set_tx_searching) = signal(false);
    let navigate = use_navigate();

    // Normalise a tx hash input: add 0x prefix if missing, lowercase
    let normalise_tx_hash = |input: &str| -> String {
        let trimmed = input.trim();
        if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
            trimmed.to_lowercase()
        } else if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            format!("0x{}", trimmed.to_lowercase())
        } else {
            trimmed.to_lowercase()
        }
    };

    // TX hash lookup function (uses spawn_local for WASM compatibility)
    let do_tx_lookup = move |hash: String| {
        let api = api.get();
        let navigate = navigate.clone();
        set_tx_searching.set(true);
        set_tx_search_error.set(None);

        leptos::task::spawn_local(async move {
            // Try all common chain IDs
            let chain_ids: &[u64] = &[1, 137, 42161, 10, 8453, 56, 43114, 250, 100, 11155111];
            for &chain_id in chain_ids {
                match api.lookup_invoice_by_tx(chain_id, &hash).await {
                    Ok(resp) => {
                        set_tx_searching.set(false);
                        navigate(
                            &format!("/evm/invoices/{}", resp.invoice.id),
                            Default::default(),
                        );
                        return;
                    }
                    Err(ApiError::Http { status: 404, .. }) => continue,
                    Err(ApiError::Http { status: 400, .. }) => {
                        set_tx_searching.set(false);
                        set_tx_search_error
                            .set(Some("Invalid transaction hash format".to_string()));
                        return;
                    }
                    Err(_) => continue,
                }
            }
            set_tx_searching.set(false);
            set_tx_search_error.set(Some("No invoice found for this transaction".to_string()));
        });
    };

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
        let stores_loaded = matches!(store_status.get(), StoresStatus::Loaded);
        let status = status_param.get();
        let currency = currency_param.get();
        let offset = current_offset.get();
        let _ = refresh.get();

        async move {
            // No store selected is "All Stores", not an error. Before the store
            // list lands, though, it is merely "not known yet" — the layout
            // auto-selects the first store once the fetch returns — so hold off
            // rather than firing an all-stores query that a moment later is
            // replaced by a store-scoped one.
            if store_id.is_none() && !stores_loaded {
                return Ok(None);
            }
            match api
                .list_invoices(
                    store_id.as_deref(),
                    status.as_deref(),
                    currency.as_deref(),
                    Some(PAGE_SIZE),
                    Some(offset),
                )
                .await
            {
                Ok(response) => Ok(Some(response)),
                // The all-stores view is admin-only server-side; everyone else
                // gets a 400. That is a "pick a store" situation, not a failure
                // to show as a raw error with a Retry that cannot work — fall
                // through to NoStoreSelected, which says exactly that (RCS-171).
                Err(ApiError::Http { status: 400, .. }) if store_id.is_none() => Ok(None),
                Err(e) => Err(e),
            }
        }
    });

    // The store column only earns its width when rows can come from different
    // stores; with one store selected every row would repeat the same name.
    let show_store = Signal::derive(move || store_ctx.selected_store_id.get().is_none());
    // The CSV export hits the same server gate as the list: with no store
    // selected it is admin-only, and a non-admin's request comes back 400. The
    // list already resolves to `None` in exactly that case - that is what puts
    // NoStoreSelected on screen - so the same signal says whether an export can
    // succeed. Offering the button there fired a request that could only fail,
    // and the failure was console-only, so to the user the button did nothing
    // at all. RCS-171 asks for non-admin behaviour to be handled gracefully
    // with no raw error; a button that silently does nothing is not that.
    let export_available = move || matches!(invoices_resource.get().as_deref(), Some(Ok(Some(_))));

    view! {
        <div class="invoices-page">
            // Page Header
            <div class="page-header-row">
                <div>
                    <h1 class="page-title">"Invoices"</h1>
                    <p class="page-description">"Create and manage payment invoices"</p>
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
                            match api.export_invoices_csv(store_id.as_deref(), status.as_deref()).await {
                                Ok(csv) => crate::pages::trigger_csv_download(&csv, "invoices.csv"),
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

                <div class="invoices-search tx-hash-search">
                    <IconSearch />
                    <input
                        type="text"
                        placeholder="Search by tx hash..."
                        prop:value=move || tx_search_input.get()
                        on:input=move |ev| {
                            set_tx_search_input.set(event_target_value(&ev));
                            set_tx_search_error.set(None);
                        }
                        on:keydown=move |ev| {
                            if ev.key() == "Enter" {
                                let raw = tx_search_input.get();
                                if !raw.trim().is_empty() {
                                    let hash = normalise_tx_hash(&raw);
                                    do_tx_lookup(hash);
                                }
                            }
                        }
                        disabled=move || tx_searching.get()
                    />
                    {move || tx_search_error.get().map(|err| view! {
                        <span class="tx-search-error">{err}</span>
                    })}
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
                    // Not an inline empty state: "no store selected" also covers
                    // stores still loading, a failed fetch, and a deliberate
                    // "All Stores" choice. NoStoreSelected tells them apart.
                    Ok(None) => view! {
                        <NoStoreSelected
                            entity="Invoices"
                            status=store_status
                            has_stores=has_stores
                            on_retry=retry_stores
                        />
                    }
                    .into_any(),
                    Ok(Some(response)) => {
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

                            view! {
                                // Invoice Table (Desktop)
                                <div class="invoices-table-container desktop-only">
                                    <table class="invoices-table">
                                        <thead>
                                            <tr>
                                                <th>"Invoice"</th>
                                                {show_store.get().then(|| view! { <th>"Store"</th> })}
                                                <th>"Amount"</th>
                                                <th>"Received"</th>
                                                <th>"Status"</th>
                                                <th>"Created"</th>
                                                <th></th>
                                            </tr>
                                        </thead>
                                        <tbody>
                                            {filtered.clone().into_iter().map(|invoice| {
                                                view! { <InvoiceRow invoice=invoice show_store=show_store.get() /> }
                                            }).collect_view()}
                                        </tbody>
                                    </table>
                                </div>

                                // Invoice Cards (Mobile)
                                <div class="invoices-cards mobile-only">
                                    {filtered.into_iter().map(|invoice| {
                                        view! { <InvoiceCard invoice=invoice show_store=show_store.get() /> }
                                    }).collect_view()}
                                </div>

                                // Pagination
                                <Pagination
                                    total=total
                                    page_size=PAGE_SIZE
                                    current_offset=offset
                                    on_page_change=move |new_offset| set_current_offset.set(new_offset)
                                    item_label="invoices"
                                />
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
