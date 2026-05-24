//! General settings tab component.

use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use crate::api::{EvmApiClient, Store, UpdateStoreRequest};
use crate::app::StoreContext;

/// General settings tab.
#[component]
pub fn GeneralTab(store: Store) -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");
    let store_ctx = use_context::<StoreContext>().expect("StoreContext must be provided");
    let navigate = use_navigate();
    let store_id = store.id.clone();

    let (name, set_name) = signal(store.name.clone());
    let (website, set_website) = signal(store.website.clone().unwrap_or_default());
    let (saving, set_saving) = signal(false);
    let (save_message, set_save_message) = signal(None::<(bool, String)>); // (is_success, message)
    let (deleting, set_deleting) = signal(false);

    let store_id_save = store_id.clone();
    let on_save = {
        let store_ctx = store_ctx.clone();
        move |_| {
            let api = api.get();
            let id = store_id_save.clone();
            let new_name = name.get_untracked();
            let new_website = website.get_untracked();
            let store_ctx = store_ctx.clone();
            set_saving.set(true);
            set_save_message.set(None);
            leptos::task::spawn_local(async move {
                let req = UpdateStoreRequest {
                    name: Some(new_name),
                    website: Some(if new_website.is_empty() {
                        String::new()
                    } else {
                        new_website
                    }),
                };
                match api.update_store(&id, &req).await {
                    Ok(_) => {
                        set_save_message.set(Some((true, "Changes saved".to_string())));
                        store_ctx.refetch_stores();
                    }
                    Err(e) => set_save_message.set(Some((false, e.to_string()))),
                }
                set_saving.set(false);
            });
        }
    };

    let store_id_delete = store_id.clone();
    let on_delete = move |_| {
        let confirmed = web_sys::window()
            .and_then(|w| {
                w.confirm_with_message(
                    "Are you sure you want to delete this store? This action cannot be undone.",
                )
                .ok()
            })
            .unwrap_or(false);
        if !confirmed {
            return;
        }
        let api = api.get();
        let id = store_id_delete.clone();
        let navigate = navigate.clone();
        let store_ctx = store_ctx.clone();
        set_deleting.set(true);
        leptos::task::spawn_local(async move {
            match api.delete_store(&id).await {
                Ok(_) => {
                    store_ctx.refetch_stores();
                    navigate("/evm/stores", Default::default());
                }
                Err(e) => {
                    web_sys::window()
                        .and_then(|w| w.alert_with_message(&format!("Delete failed: {}", e)).ok());
                }
            }
            set_deleting.set(false);
        });
    };

    view! {
        <div class="store-tab-general">
            <div class="detail-card">
                <div class="detail-card-header">
                    <h3>"Store Information"</h3>
                </div>
                <div class="detail-card-body">
                    {move || save_message.get().map(|(success, msg)| {
                        let class = if success { "color: var(--color-success)" } else { "color: var(--color-error)" };
                        view! { <p style=class>{msg}</p> }
                    })}

                    <div class="form-group">
                        <label class="form-label">"Store Name"</label>
                        <input
                            type="text"
                            class="form-input"
                            prop:value=move || name.get()
                            on:input=move |ev| set_name.set(event_target_value(&ev))
                        />
                        <p class="form-help">"The name displayed to your customers"</p>
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Website"</label>
                        <input
                            type="url"
                            class="form-input"
                            placeholder="https://example.com"
                            prop:value=move || website.get()
                            on:input=move |ev| set_website.set(event_target_value(&ev))
                        />
                        <p class="form-help">"Your store's website URL (optional)"</p>
                    </div>

                    <div class="form-actions">
                        <button
                            class="btn btn-primary btn-sm"
                            on:click=on_save
                            disabled=move || saving.get()
                        >
                            {move || if saving.get() { "Saving..." } else { "Save changes" }}
                        </button>
                    </div>
                </div>
            </div>

            <div class="detail-card detail-card-danger">
                <div class="detail-card-header">
                    <h3>"Danger Zone"</h3>
                </div>
                <div class="detail-card-body">
                    <div class="danger-action">
                        <div class="danger-action-info">
                            <span class="danger-action-title">"Delete this store"</span>
                            <span class="danger-action-desc">"Permanently delete this store and all its data"</span>
                        </div>
                        <button
                            class="btn btn-danger btn-sm"
                            on:click=on_delete
                            disabled=move || deleting.get()
                        >
                            {move || if deleting.get() { "Deleting..." } else { "Delete store" }}
                        </button>
                    </div>
                </div>
            </div>
        </div>
    }
}
