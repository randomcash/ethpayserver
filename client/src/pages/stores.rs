//! Store management pages.

use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use ui_kit::components::{
    Button, ButtonVariant, Card, CardWithHeader, Grid, PageHeader, Skeleton, Stack,
};

use crate::api::Store;

/// Stores list page.
#[component]
pub fn StoresPage() -> impl IntoView {
    let stores = Resource::new(|| (), |_| async {
        // TODO: Fetch from API
        Ok::<_, String>(vec![
            Store {
                id: "store_001".to_string(),
                name: "My Online Shop".to_string(),
                website: Some("https://example.com".to_string()),
                default_currency: "USD".to_string(),
                enabled_networks: vec![1, 137, 42161],
                created_at: "2024-01-01T00:00:00Z".to_string(),
            },
        ])
    });

    view! {
        <div class="evm-page">
            <PageHeader
                title="Stores"
                description="Manage your payment stores"
                actions=view! {
                    <Button variant=ButtonVariant::Primary>
                        "Create Store"
                    </Button>
                }.into_any()
            />

            <Suspense fallback=move || view! { <StoreListSkeleton /> }>
                {move || {
                    stores.get().map(|result| {
                        match result {
                            Ok(list) => view! { <StoreList stores=list /> }.into_any(),
                            Err(e) => view! {
                                <Card>
                                    <p class="text-error">"Error loading stores: " {e}</p>
                                </Card>
                            }.into_any(),
                        }
                    })
                }}
            </Suspense>
        </div>
    }
}

#[component]
fn StoreList(stores: Vec<Store>) -> impl IntoView {
    if stores.is_empty() {
        return view! {
            <Card>
                <div class="evm-empty-state">
                    <p>"No stores configured"</p>
                    <Button variant=ButtonVariant::Primary>
                        "Create your first store"
                    </Button>
                </div>
            </Card>
        }.into_any();
    }

    view! {
        <Grid cols=2 gap="md">
            {stores.into_iter().map(|store| {
                view! { <StoreCard store=store /> }
            }).collect_view()}
        </Grid>
    }.into_any()
}

#[component]
fn StoreCard(store: Store) -> impl IntoView {
    use ui_kit::components::crypto::NetworkBadge;

    let store_name = store.name.clone();
    let store_id = store.id.clone();
    let default_currency = store.default_currency.clone();

    view! {
        <Card class="evm-store-card">
            <div class="evm-store-header">
                <h3 class="evm-store-name">
                    <a href=format!("/evm/stores/{}", store_id)>{store_name}</a>
                </h3>
                {store.website.clone().map(|url| {
                    let url_display = url.clone();
                    view! {
                        <a href=url target="_blank" class="evm-store-website">
                            {url_display}
                        </a>
                    }
                })}
            </div>

            <div class="evm-store-networks">
                <span class="evm-store-networks-label">"Enabled Networks"</span>
                <div class="evm-store-network-badges">
                    {store.enabled_networks.iter().filter_map(|&id| {
                        chain_id_to_network(id).map(|network| {
                            view! { <NetworkBadge network=network /> }
                        })
                    }).collect_view()}
                </div>
            </div>

            <div class="evm-store-footer">
                <span class="evm-store-currency">
                    "Default: " {default_currency}
                </span>
                <Button variant=ButtonVariant::Ghost>
                    "Configure"
                </Button>
            </div>
        </Card>
    }
}

#[component]
fn StoreListSkeleton() -> impl IntoView {
    view! {
        <Grid cols=2 gap="md">
            <Skeleton height="200px" />
            <Skeleton height="200px" />
        </Grid>
    }
}

/// Store detail page.
#[component]
pub fn StoreDetailPage() -> impl IntoView {
    let params = use_params_map();
    let store_id = move || params.get().get("id").unwrap_or_default();

    let store = Resource::new(
        move || store_id(),
        |id| async move {
            // TODO: Fetch from API
            Ok::<_, String>(Store {
                id,
                name: "My Online Shop".to_string(),
                website: Some("https://example.com".to_string()),
                default_currency: "USD".to_string(),
                enabled_networks: vec![1, 137, 42161],
                created_at: "2024-01-01T00:00:00Z".to_string(),
            })
        },
    );

    view! {
        <div class="evm-page">
            <Suspense fallback=move || view! { <Skeleton height="400px" /> }>
                {move || {
                    store.get().map(|result| {
                        match result {
                            Ok(s) => view! { <StoreDetail store=s /> }.into_any(),
                            Err(e) => view! {
                                <Card>
                                    <p class="text-error">"Error loading store: " {e}</p>
                                </Card>
                            }.into_any(),
                        }
                    })
                }}
            </Suspense>
        </div>
    }
}

#[component]
fn StoreDetail(store: Store) -> impl IntoView {
    use ui_kit::components::{Checkbox, Input};
    use ui_kit::components::crypto::NetworkBadge;

    let name = RwSignal::new(store.name.clone());
    let website = RwSignal::new(store.website.clone().unwrap_or_default());

    view! {
        <Stack gap="lg">
            <PageHeader title="Store Settings" />

            <Grid cols=2 gap="lg">
                <CardWithHeader title="General">
                    <Stack gap="md">
                        <Input
                            value=name
                            label="Store Name"
                            placeholder="My Store"
                        />
                        <Input
                            value=website
                            label="Website"
                            placeholder="https://example.com"
                        />
                    </Stack>
                </CardWithHeader>

                <CardWithHeader title="Networks">
                    <Stack gap="sm">
                        {[
                            (1, "Ethereum"),
                            (137, "Polygon"),
                            (42161, "Arbitrum"),
                            (10, "Optimism"),
                            (8453, "Base"),
                        ].into_iter().map(|(chain_id, _name)| {
                            let enabled = RwSignal::new(store.enabled_networks.contains(&chain_id));
                            let network = chain_id_to_network(chain_id).unwrap_or(types::Network::Ethereum);
                            view! {
                                <div class="evm-network-toggle">
                                    <Checkbox checked=enabled />
                                    <NetworkBadge network=network />
                                </div>
                            }
                        }).collect_view()}
                    </Stack>
                </CardWithHeader>
            </Grid>

            <div class="evm-page-actions">
                <Button variant=ButtonVariant::Primary>
                    "Save Changes"
                </Button>
                <Button variant=ButtonVariant::Ghost>
                    "Cancel"
                </Button>
            </div>
        </Stack>
    }
}

fn chain_id_to_network(chain_id: u64) -> Option<types::Network> {
    match chain_id {
        1 => Some(types::Network::Ethereum),
        137 => Some(types::Network::Polygon),
        42161 => Some(types::Network::Arbitrum),
        10 => Some(types::Network::Optimism),
        8453 => Some(types::Network::Base),
        43114 => Some(types::Network::Avalanche),
        56 => Some(types::Network::BinanceSmartChain),
        324 => Some(types::Network::ZkSync),
        59144 => Some(types::Network::Linea),
        534352 => Some(types::Network::Scroll),
        _ => None,
    }
}

/// Store page styles.
pub const STORE_STYLES: &str = r#"
.evm-store-card {
    padding: var(--ps-spacing-lg);
}

.evm-store-header {
    margin-bottom: var(--ps-spacing-md);
}

.evm-store-name {
    margin: 0 0 var(--ps-spacing-xs) 0;
    font-size: var(--ps-font-lg);
}

.evm-store-name a {
    color: var(--ps-text);
    text-decoration: none;
}

.evm-store-name a:hover {
    color: var(--ps-primary);
}

.evm-store-website {
    font-size: var(--ps-font-sm);
    color: var(--ps-text-muted);
    text-decoration: none;
}

.evm-store-website:hover {
    text-decoration: underline;
}

.evm-store-networks {
    padding: var(--ps-spacing-md) 0;
    border-top: 1px solid var(--ps-border);
    border-bottom: 1px solid var(--ps-border);
}

.evm-store-networks-label {
    display: block;
    font-size: var(--ps-font-sm);
    color: var(--ps-text-muted);
    margin-bottom: var(--ps-spacing-sm);
}

.evm-store-network-badges {
    display: flex;
    flex-wrap: wrap;
    gap: var(--ps-spacing-xs);
}

.evm-store-footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-top: var(--ps-spacing-md);
}

.evm-store-currency {
    font-size: var(--ps-font-sm);
    color: var(--ps-text-muted);
}

.evm-network-toggle {
    display: flex;
    align-items: center;
    gap: var(--ps-spacing-sm);
    padding: var(--ps-spacing-xs) 0;
}
"#;
