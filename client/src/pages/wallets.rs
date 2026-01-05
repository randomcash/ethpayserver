//! Wallet management page.

use leptos::prelude::*;
use ui_kit::components::{
    Button, ButtonVariant, Card, PageHeader, Skeleton, Stack,
};
use ui_kit::components::crypto::{Address, NetworkBadge};

use crate::api::Wallet;

/// Wallets page.
#[component]
pub fn WalletsPage() -> impl IntoView {
    let wallets = Resource::new(|| (), |_| async {
        // TODO: Fetch from API
        Ok::<_, String>(vec![
            Wallet {
                id: "wallet_001".to_string(),
                name: "Main Wallet".to_string(),
                address: "0x742d35Cc6634C0532925a3b844Bc9e7595f0Ab3D".to_string(),
                derivation_path: "m/44'/60'/0'/0/0".to_string(),
                enabled_chains: vec![1, 137, 42161],
                created_at: "2024-01-01T00:00:00Z".to_string(),
            },
        ])
    });

    view! {
        <div class="evm-page">
            <PageHeader
                title="Wallets"
                description="Configure your payment wallets"
                actions=view! {
                    <Button variant=ButtonVariant::Primary>
                        "Add Wallet"
                    </Button>
                }.into_any()
            />

            <Suspense fallback=move || view! { <WalletListSkeleton /> }>
                {move || {
                    wallets.get().map(|result| {
                        match result {
                            Ok(list) => view! { <WalletList wallets=list /> }.into_any(),
                            Err(e) => view! {
                                <Card>
                                    <p class="text-error">"Error loading wallets: " {e}</p>
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
fn WalletList(wallets: Vec<Wallet>) -> impl IntoView {
    if wallets.is_empty() {
        return view! {
            <Card>
                <div class="evm-empty-state">
                    <p>"No wallets configured"</p>
                    <p class="evm-empty-hint">"Add a wallet to start receiving payments"</p>
                    <Button variant=ButtonVariant::Primary>
                        "Add Wallet"
                    </Button>
                </div>
            </Card>
        }.into_any();
    }

    view! {
        <Stack gap="md">
            {wallets.into_iter().map(|wallet| {
                view! { <WalletCard wallet=wallet /> }
            }).collect_view()}
        </Stack>
    }.into_any()
}

#[component]
fn WalletCard(wallet: Wallet) -> impl IntoView {
    let wallet_name = wallet.name.clone();
    let derivation_path = wallet.derivation_path.clone();

    view! {
        <Card class="evm-wallet-card">
            <div class="evm-wallet-header">
                <div>
                    <h3 class="evm-wallet-name">{wallet_name}</h3>
                    <div class="evm-wallet-address">
                        <Address address=wallet.address.clone() prefix_len=10 suffix_len=8 />
                    </div>
                </div>
                <Button variant=ButtonVariant::Ghost>
                    "Edit"
                </Button>
            </div>

            <div class="evm-wallet-details">
                <div class="evm-wallet-detail">
                    <span class="evm-wallet-detail-label">"Derivation Path"</span>
                    <code class="evm-wallet-path">{derivation_path}</code>
                </div>
            </div>

            <div class="evm-wallet-chains">
                <span class="evm-wallet-chains-label">"Enabled Chains"</span>
                <div class="evm-wallet-chain-badges">
                    {wallet.enabled_chains.iter().filter_map(|&id| {
                        chain_id_to_network(id).map(|network| {
                            view! { <NetworkBadge network=network /> }
                        })
                    }).collect_view()}
                </div>
            </div>
        </Card>
    }
}

#[component]
fn WalletListSkeleton() -> impl IntoView {
    view! {
        <Stack gap="md">
            <Skeleton height="180px" />
            <Skeleton height="180px" />
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

/// Wallet page styles.
pub const WALLET_STYLES: &str = r#"
.evm-wallet-card {
    padding: var(--ps-spacing-lg);
}

.evm-wallet-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    margin-bottom: var(--ps-spacing-md);
}

.evm-wallet-name {
    margin: 0 0 var(--ps-spacing-xs) 0;
    font-size: var(--ps-font-lg);
    font-weight: 600;
}

.evm-wallet-address {
    font-family: monospace;
}

.evm-wallet-details {
    padding: var(--ps-spacing-md) 0;
    border-top: 1px solid var(--ps-border);
    border-bottom: 1px solid var(--ps-border);
}

.evm-wallet-detail {
    display: flex;
    justify-content: space-between;
    align-items: center;
}

.evm-wallet-detail-label {
    font-size: var(--ps-font-sm);
    color: var(--ps-text-muted);
}

.evm-wallet-path {
    font-size: var(--ps-font-sm);
    padding: var(--ps-spacing-xs) var(--ps-spacing-sm);
    background-color: var(--ps-background);
    border-radius: var(--ps-radius-sm);
}

.evm-wallet-chains {
    margin-top: var(--ps-spacing-md);
}

.evm-wallet-chains-label {
    display: block;
    font-size: var(--ps-font-sm);
    color: var(--ps-text-muted);
    margin-bottom: var(--ps-spacing-sm);
}

.evm-wallet-chain-badges {
    display: flex;
    flex-wrap: wrap;
    gap: var(--ps-spacing-xs);
}

.evm-empty-hint {
    font-size: var(--ps-font-sm);
    color: var(--ps-text-muted);
}
"#;
