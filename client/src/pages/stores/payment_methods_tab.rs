//! Payment methods tab component.

use leptos::prelude::*;

use crate::api::{
    CreatePaymentMethodRequest, EvmApiClient, StorePaymentMethod, UpdatePaymentMethodRequest,
};
use crate::util::chain_name;

use super::IconPlus;

/// Payment methods tab.
#[component]
pub fn PaymentMethodsTab(store_id: String) -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");

    // Fetch payment methods from API
    let (refresh_counter, set_refresh_counter) = signal(0u32);
    let store_id_fetch = store_id.clone();
    let methods_resource = LocalResource::new(move || {
        let api = api.get();
        let id = store_id_fetch.clone();
        refresh_counter.get();
        async move { api.list_payment_methods(&id).await }
    });

    // Create form state
    let (show_create_form, set_show_create_form) = signal(false);
    let (new_chain_id, set_new_chain_id) = signal("11155111".to_string());
    let (new_asset_symbol, set_new_asset_symbol) = signal(String::new());
    let (new_token_address, set_new_token_address) = signal(String::new());
    let (new_decimals, set_new_decimals) = signal("18".to_string());
    let (new_xpub, set_new_xpub) = signal(String::new());
    let (creating, set_creating) = signal(false);
    let (create_error, set_create_error) = signal(None::<String>);

    let store_id_create = store_id.clone();
    let on_create = move |_| {
        let xpub = new_xpub.get_untracked();
        let symbol = new_asset_symbol.get_untracked();
        if xpub.trim().is_empty() || symbol.trim().is_empty() {
            set_create_error.set(Some("Asset symbol and xpub are required".to_string()));
            return;
        }
        let chain_id: u64 = match new_chain_id.get_untracked().parse() {
            Ok(v) => v,
            Err(_) => {
                set_create_error.set(Some("Invalid chain ID".to_string()));
                return;
            }
        };
        let decimals: u8 = match new_decimals.get_untracked().parse() {
            Ok(v) => v,
            Err(_) => {
                set_create_error.set(Some("Invalid decimals".to_string()));
                return;
            }
        };
        let token_address = {
            let addr = new_token_address.get_untracked();
            if addr.trim().is_empty() {
                None
            } else {
                Some(addr.trim().to_string())
            }
        };
        let api = api.get();
        let sid = store_id_create.clone();
        set_creating.set(true);
        set_create_error.set(None);
        leptos::task::spawn_local(async move {
            let req = CreatePaymentMethodRequest {
                chain_id,
                token_address,
                asset_symbol: symbol.trim().to_string(),
                decimals,
                xpub: xpub.trim().to_string(),
            };
            match api.create_payment_method(&sid, &req).await {
                Ok(_) => {
                    set_show_create_form.set(false);
                    set_new_asset_symbol.set(String::new());
                    set_new_token_address.set(String::new());
                    set_new_xpub.set(String::new());
                    set_new_decimals.set("18".to_string());
                    set_refresh_counter.update(|c| *c += 1);
                }
                Err(e) => {
                    set_create_error.set(Some(e.to_string()));
                }
            }
            set_creating.set(false);
        });
    };

    view! {
        <div class="store-tab-payment-methods">
            <div class="section-header">
                <div>
                    <h3 class="section-title">"Payment Methods"</h3>
                    <Suspense fallback=|| view! { <p class="section-desc">"Loading..."</p> }>
                        {move || methods_resource.get().map(|result| match &*result {
                            Ok(methods) => {
                                let enabled = methods.iter().filter(|m| m.enabled).count();
                                let total = methods.len();
                                view! { <p class="section-desc">{enabled}" of "{total}" methods enabled"</p> }.into_any()
                            }
                            Err(_) => view! { <p class="section-desc">""</p> }.into_any(),
                        })}
                    </Suspense>
                </div>
                <button
                    class="btn btn-primary btn-sm"
                    on:click=move |_| set_show_create_form.update(|v| *v = !*v)
                >
                    <IconPlus />
                    "Add method"
                </button>
            </div>

            // Create form
            {move || show_create_form.get().then(|| {
                let on_create = on_create.clone();
                view! {
                    <div class="detail-card" style="margin-bottom: 1.5rem;">
                        <div class="detail-card-header">
                            <h3>"Add Payment Method"</h3>
                        </div>
                        <div class="detail-card-body">
                            {move || create_error.get().map(|err| view! {
                                <div class="form-error" style="color: var(--color-error); margin-bottom: 1rem; padding: 0.5rem; background: var(--color-error-bg, rgba(239,68,68,0.1)); border-radius: 4px;">
                                    {err}
                                </div>
                            })}
                            <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 1rem;">
                                <div>
                                    <label class="form-label">"Network (Chain ID)"</label>
                                    <select
                                        class="form-select"
                                        on:change=move |ev| set_new_chain_id.set(event_target_value(&ev))
                                        prop:value=move || new_chain_id.get()
                                    >
                                        <option value="1">"Ethereum (1)"</option>
                                        <option value="137">"Polygon (137)"</option>
                                        <option value="42161">"Arbitrum (42161)"</option>
                                        <option value="10">"Optimism (10)"</option>
                                        <option value="8453">"Base (8453)"</option>
                                        <option value="56">"BSC (56)"</option>
                                        <option value="43114">"Avalanche (43114)"</option>
                                        <option value="324">"zkSync (324)"</option>
                                        <option value="59144">"Linea (59144)"</option>
                                        <option value="534352">"Scroll (534352)"</option>
                                        <option value="100">"Gnosis (100)"</option>
                                        <option value="250">"Fantom (250)"</option>
                                        <option value="11155111" selected>"Sepolia (11155111)"</option>
                                    </select>
                                </div>
                                <div>
                                    <label class="form-label">"Asset Symbol"</label>
                                    <input
                                        type="text"
                                        class="form-input"
                                        placeholder="ETH"
                                        prop:value=move || new_asset_symbol.get()
                                        on:input=move |ev| set_new_asset_symbol.set(event_target_value(&ev))
                                    />
                                </div>
                                <div>
                                    <label class="form-label">"Token Address (ERC20 only)"</label>
                                    <input
                                        type="text"
                                        class="form-input"
                                        placeholder="0x... (leave empty for native asset)"
                                        prop:value=move || new_token_address.get()
                                        on:input=move |ev| set_new_token_address.set(event_target_value(&ev))
                                    />
                                </div>
                                <div>
                                    <label class="form-label">"Decimals"</label>
                                    <input
                                        type="number"
                                        class="form-input"
                                        prop:value=move || new_decimals.get()
                                        on:input=move |ev| set_new_decimals.set(event_target_value(&ev))
                                    />
                                </div>
                            </div>
                            <div class="form-group" style="margin-top: 1rem;">
                                <label class="form-label">"Extended Public Key (xpub)"</label>
                                <input
                                    type="text"
                                    class="form-input"
                                    placeholder="xpub..."
                                    prop:value=move || new_xpub.get()
                                    on:input=move |ev| set_new_xpub.set(event_target_value(&ev))
                                />
                            </div>
                            <div class="form-actions" style="margin-top: 1rem; display: flex; gap: 0.5rem;">
                                <button
                                    class="btn btn-primary btn-sm"
                                    on:click=on_create
                                    disabled=move || creating.get()
                                >
                                    {move || if creating.get() { "Creating..." } else { "Create" }}
                                </button>
                                <button
                                    class="btn btn-secondary btn-sm"
                                    on:click=move |_| set_show_create_form.set(false)
                                >
                                    "Cancel"
                                </button>
                            </div>
                        </div>
                    </div>
                }
            })}

            <Suspense fallback=move || view! {
                <div style="text-align: center; padding: 2rem; color: var(--text-muted);">
                    "Loading payment methods..."
                </div>
            }>
                {move || {
                    let store_id = store_id.clone();
                    methods_resource.get().map(move |result| match &*result {
                        Ok(methods) if methods.is_empty() => view! {
                            <div class="empty-state" style="text-align: center; padding: 3rem; color: var(--text-muted);">
                                <p style="font-size: 1.1rem;">"No payment methods configured"</p>
                                <p style="margin-top: 0.5rem;">"Add a payment method to start accepting payments"</p>
                            </div>
                        }.into_any(),
                        Ok(methods) => view! {
                            <div class="payment-methods-table-container">
                                <table class="payment-methods-table">
                                    <thead>
                                        <tr>
                                            <th>"Asset"</th>
                                            <th>"Network"</th>
                                            <th>"Type"</th>
                                            <th>"Derivation Index"</th>
                                            <th>"Status"</th>
                                            <th></th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {methods.iter().map(|method| {
                                            let method = method.clone();
                                            let sid = store_id.clone();
                                            view! { <PaymentMethodRow method=method store_id=sid set_refresh_counter=set_refresh_counter /> }
                                        }).collect_view()}
                                    </tbody>
                                </table>
                            </div>
                        }.into_any(),
                        Err(e) => view! {
                            <div style="text-align: center; padding: 2rem; color: var(--color-error);">
                                <p>"Failed to load payment methods: "{e.to_string()}</p>
                                <button
                                    class="btn btn-secondary btn-sm"
                                    style="margin-top: 1rem;"
                                    on:click=move |_| set_refresh_counter.update(|c| *c += 1)
                                >
                                    "Retry"
                                </button>
                            </div>
                        }.into_any(),
                    })
                }}
            </Suspense>
        </div>
    }
}

/// Payment method table row.
#[component]
fn PaymentMethodRow(
    method: StorePaymentMethod,
    store_id: String,
    set_refresh_counter: WriteSignal<u32>,
) -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");
    let network = chain_name(method.chain_id);
    let asset_type = if method.token_address.is_some() {
        "ERC20"
    } else {
        "Native"
    };
    let method_id = method.id.clone();
    let method_id_toggle = method.id.clone();
    let is_enabled = method.enabled;

    let (toggling, set_toggling) = signal(false);
    let (deleting, set_deleting) = signal(false);

    let on_toggle = {
        let store_id = store_id.clone();
        move |_| {
            let api = api.get();
            let sid = store_id.clone();
            let mid = method_id_toggle.clone();
            set_toggling.set(true);
            leptos::task::spawn_local(async move {
                let req = UpdatePaymentMethodRequest {
                    enabled: Some(!is_enabled),
                    xpub: None,
                };
                let _ = api.update_payment_method(&sid, &mid, &req).await;
                set_toggling.set(false);
                set_refresh_counter.update(|c| *c += 1);
            });
        }
    };

    let on_delete = {
        let store_id = store_id.clone();
        move |_| {
            let api = api.get();
            let sid = store_id.clone();
            let mid = method_id.clone();
            set_deleting.set(true);
            leptos::task::spawn_local(async move {
                let _ = api.delete_payment_method(&sid, &mid).await;
                set_deleting.set(false);
                set_refresh_counter.update(|c| *c += 1);
            });
        }
    };

    let status_class = if method.enabled {
        "badge badge-success"
    } else {
        "badge badge-neutral"
    };
    let status_label = if method.enabled {
        "Enabled"
    } else {
        "Disabled"
    };

    view! {
        <tr class="payment-method-row">
            <td>
                <div class="payment-method-asset">
                    <span class="payment-method-symbol">{method.asset_symbol}</span>
                </div>
            </td>
            <td>
                <span class="payment-method-network">{network}</span>
            </td>
            <td>
                <span class="payment-method-type">{asset_type}</span>
            </td>
            <td>
                <code class="payment-method-index">{method.derivation_index}</code>
            </td>
            <td>
                <button
                    class=status_class
                    style="cursor: pointer; border: none; font: inherit;"
                    on:click=on_toggle
                    disabled=move || toggling.get()
                >
                    {move || if toggling.get() { "..." } else { status_label }}
                </button>
            </td>
            <td>
                <button
                    class="btn btn-ghost btn-sm"
                    style="color: var(--color-error);"
                    on:click=on_delete
                    disabled=move || deleting.get()
                >
                    {move || if deleting.get() { "..." } else { "Delete" }}
                </button>
            </td>
        </tr>
    }
}
