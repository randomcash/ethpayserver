//! Store list page and store card component.

use leptos::prelude::*;
use leptos_router::components::A;

use crate::api::{CreateStoreRequest, EvmApiClient, Store};
use crate::app::StoreContext;

use super::{
    IconCalendar, IconChevronRight, IconGlobe, IconPlus, IconStore, event_target_checked,
    format_date,
};

/// Stores list page.
#[component]
pub fn StoresPage() -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");
    let store_ctx = use_context::<StoreContext>().expect("StoreContext must be provided");

    // Fetch stores from API
    let (refresh_counter, set_refresh_counter) = signal(0u32);
    let stores_resource = LocalResource::new(move || {
        let api = api.get();
        refresh_counter.get(); // Track to allow manual refresh
        async move { api.list_stores().await }
    });

    // Create store form state
    let (show_create_form, set_show_create_form) = signal(false);
    let (new_store_name, set_new_store_name) = signal(String::new());
    let (creating, set_creating) = signal(false);
    let (create_error, set_create_error) = signal(None::<String>);

    let on_create_store = {
        let store_ctx = store_ctx.clone();
        move |_| {
            let name = new_store_name.get_untracked();
            if name.trim().is_empty() {
                return;
            }
            let api = api.get();
            let store_ctx = store_ctx.clone();
            set_creating.set(true);
            set_create_error.set(None);
            leptos::task::spawn_local(async move {
                let req = CreateStoreRequest {
                    name: name.trim().to_string(),
                    website: None,
                };
                match api.create_store(&req).await {
                    Ok(_) => {
                        set_new_store_name.set(String::new());
                        set_show_create_form.set(false);
                        set_refresh_counter.update(|c| *c += 1);
                        store_ctx.refetch_stores();
                    }
                    Err(e) => {
                        set_create_error.set(Some(e.to_string()));
                    }
                }
                set_creating.set(false);
            });
        }
    };

    // Filter to show only active stores by default
    let (show_archived, set_show_archived) = signal(false);

    view! {
        <div class="stores-page">
            // Page Header
            <div class="page-header-row">
                <div>
                    <h1 class="page-title">"Stores"</h1>
                    <p class="page-description">"Manage your payment stores and configurations"</p>
                </div>
                <div class="page-actions">
                    <button
                        class="btn btn-primary btn-sm"
                        on:click=move |_| set_show_create_form.update(|v| *v = !*v)
                    >
                        <IconPlus />
                        "Create store"
                    </button>
                </div>
            </div>

            // Create store form
            {move || show_create_form.get().then(|| view! {
                <div class="detail-card" style="margin-bottom: 1.5rem;">
                    <div class="detail-card-header">
                        <h3>"New Store"</h3>
                    </div>
                    <div class="detail-card-body">
                        {move || create_error.get().map(|e| view! {
                            <div class="form-error" style="color: var(--color-error); margin-bottom: 0.75rem;">{e}</div>
                        })}
                        <div class="form-group">
                            <label class="form-label">"Store Name"</label>
                            <input
                                type="text"
                                class="form-input"
                                placeholder="My Store"
                                prop:value=move || new_store_name.get()
                                on:input=move |ev| set_new_store_name.set(event_target_value(&ev))
                                on:keydown={
                                    let store_ctx = store_ctx.clone();
                                    move |ev: web_sys::KeyboardEvent| {
                                        if ev.key() == "Enter" {
                                            ev.prevent_default();
                                            let name = new_store_name.get_untracked();
                                            if name.trim().is_empty() {
                                                return;
                                            }
                                            let api = api.get();
                                            let store_ctx = store_ctx.clone();
                                            set_creating.set(true);
                                            set_create_error.set(None);
                                            leptos::task::spawn_local(async move {
                                                let req = CreateStoreRequest {
                                                    name: name.trim().to_string(),
                                                    website: None,
                                                };
                                                match api.create_store(&req).await {
                                                    Ok(_) => {
                                                        set_new_store_name.set(String::new());
                                                        set_show_create_form.set(false);
                                                        set_refresh_counter.update(|c| *c += 1);
                                                        store_ctx.refetch_stores();
                                                    }
                                                    Err(e) => {
                                                        set_create_error.set(Some(e.to_string()));
                                                    }
                                                }
                                                set_creating.set(false);
                                            });
                                        }
                                    }
                                }
                            />
                        </div>
                        <div class="form-actions">
                            <button
                                class="btn btn-primary btn-sm"
                                on:click=on_create_store.clone()
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
            })}

            // Toolbar
            <div class="stores-toolbar">
                <label class="checkbox-label">
                    <input
                        type="checkbox"
                        prop:checked=move || show_archived.get()
                        on:change=move |ev| set_show_archived.set(event_target_checked(&ev))
                    />
                    <span>"Show archived"</span>
                </label>
            </div>

            // Stores Grid
            <Suspense fallback=move || view! {
                <div class="stores-loading" style="text-align: center; padding: 3rem; color: var(--text-muted);">
                    "Loading stores..."
                </div>
            }>
                {move || stores_resource.get().map(|result| match &*result {
                    Ok(stores) => {
                        if stores.is_empty() {
                            view! {
                                <div class="stores-empty" style="text-align: center; padding: 3rem;">
                                    <IconStore />
                                    <h3 style="margin-top: 1rem;">"No stores yet"</h3>
                                    <p style="color: var(--text-muted);">"Create your first store to start accepting payments"</p>
                                </div>
                            }.into_any()
                        } else {
                            let show = show_archived.get();
                            view! {
                                <div class="stores-grid">
                                    {stores.iter()
                                        .filter(|s| show || !s.archived)
                                        .cloned()
                                        .map(|store| view! { <StoreCard store=store /> })
                                        .collect_view()
                                    }
                                </div>
                            }.into_any()
                        }
                    }
                    Err(e) => view! {
                        <div class="stores-error" style="text-align: center; padding: 3rem; color: var(--color-error);">
                            <p>"Failed to load stores: "{e.to_string()}</p>
                            <button
                                class="btn btn-secondary btn-sm"
                                style="margin-top: 1rem;"
                                on:click=move |_| set_refresh_counter.update(|c| *c += 1)
                            >
                                "Retry"
                            </button>
                        </div>
                    }.into_any(),
                })}
            </Suspense>
        </div>
    }
}

/// Store card component.
#[component]
fn StoreCard(store: Store) -> impl IntoView {
    let store_id = store.id.clone();
    let store_link = store.id.clone();
    let store_name = store.name.clone();
    let created_display = format_date(&store.created_at);

    view! {
        <A href=format!("/evm/stores/{}", store_link) attr:class="store-card">
            <div class="store-card-header">
                <div class="store-card-icon">
                    <IconStore />
                </div>
                <div class="store-card-title">
                    <h3 class="store-card-name">{store_name}</h3>
                    {store.archived.then(|| view! {
                        <span class="badge badge-neutral">"Archived"</span>
                    })}
                </div>
            </div>

            <div class="store-card-body">
                {store.website.clone().map(|url| view! {
                    <div class="store-card-row">
                        <IconGlobe />
                        <span class="store-card-website">{url}</span>
                    </div>
                })}
                <div class="store-card-row">
                    <IconCalendar />
                    <span class="store-card-date">"Created "{created_display}</span>
                </div>
            </div>

            <div class="store-card-footer">
                <span class="store-card-id">{store_id}</span>
                <IconChevronRight />
            </div>
        </A>
    }
}
