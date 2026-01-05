//! Invoice management pages.

use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use ui_kit::components::{
    Button, ButtonVariant, Card, CardWithHeader, Grid, PageHeader, Skeleton, Stack,
};
use ui_kit::components::crypto::{Address, AmountWithFiat, NetworkBadge, StatusBadge};

use crate::api::Invoice;

/// Invoice list page.
#[component]
pub fn InvoicesPage() -> impl IntoView {
    let invoices = Resource::new(|| (), |_| async {
        // TODO: Fetch from API
        Ok::<_, String>(vec![
            Invoice {
                id: "inv_001".to_string(),
                store_id: "store_001".to_string(),
                amount: "100.00".to_string(),
                currency: "USD".to_string(),
                crypto_amount: Some("0.05".to_string()),
                crypto_currency: Some("ETH".to_string()),
                status: "pending".to_string(),
                payment_address: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f0Ab3D".to_string()),
                chain_id: Some(1),
                created_at: "2024-01-15T10:30:00Z".to_string(),
                expires_at: Some("2024-01-15T11:30:00Z".to_string()),
            },
            Invoice {
                id: "inv_002".to_string(),
                store_id: "store_001".to_string(),
                amount: "250.00".to_string(),
                currency: "USD".to_string(),
                crypto_amount: Some("0.125".to_string()),
                crypto_currency: Some("ETH".to_string()),
                status: "paid".to_string(),
                payment_address: Some("0x8ba1f109551bD432803012645Ac136ddd64DBA72".to_string()),
                chain_id: Some(137),
                created_at: "2024-01-14T09:00:00Z".to_string(),
                expires_at: None,
            },
        ])
    });

    view! {
        <div class="evm-page">
            <PageHeader
                title="Invoices"
                description="Manage your payment invoices"
                actions=view! {
                    <Button variant=ButtonVariant::Primary>
                        "Create Invoice"
                    </Button>
                }.into_any()
            />

            <Suspense fallback=move || view! { <InvoiceListSkeleton /> }>
                {move || {
                    invoices.get().map(|result| {
                        match result {
                            Ok(list) => view! { <InvoiceList invoices=list /> }.into_any(),
                            Err(e) => view! {
                                <Card>
                                    <p class="text-error">"Error loading invoices: " {e}</p>
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
fn InvoiceList(invoices: Vec<Invoice>) -> impl IntoView {
    if invoices.is_empty() {
        return view! {
            <Card>
                <div class="evm-empty-state">
                    <p>"No invoices yet"</p>
                    <Button variant=ButtonVariant::Primary>
                        "Create your first invoice"
                    </Button>
                </div>
            </Card>
        }.into_any();
    }

    view! {
        <div class="evm-invoice-list">
            {invoices.into_iter().map(|invoice| {
                view! { <InvoiceListItem invoice=invoice /> }
            }).collect_view()}
        </div>
    }.into_any()
}

#[component]
fn InvoiceListItem(invoice: Invoice) -> impl IntoView {
    let network = invoice.chain_id
        .and_then(|id| chain_id_to_network(id))
        .unwrap_or(types::Network::Ethereum);

    let invoice_id = invoice.id.clone();
    let invoice_id_link = invoice.id.clone();

    view! {
        <Card class="evm-invoice-item">
            <div class="evm-invoice-item-header">
                <div class="evm-invoice-item-id">
                    <a href=format!("/evm/invoices/{}", invoice_id_link)>{invoice_id}</a>
                </div>
                <StatusBadge status=invoice.status.clone() />
            </div>
            <div class="evm-invoice-item-body">
                <div class="evm-invoice-item-amount">
                    {invoice.crypto_amount.clone().map(|amt| {
                        let symbol = invoice.crypto_currency.clone().unwrap_or("ETH".to_string());
                        let fiat = format!("{} {}", invoice.amount, invoice.currency);
                        view! {
                            <AmountWithFiat
                                crypto_amount=amt
                                crypto_symbol=symbol
                                fiat_amount=fiat
                            />
                        }
                    })}
                </div>
                <div class="evm-invoice-item-network">
                    <NetworkBadge network=network />
                </div>
            </div>
            {invoice.payment_address.clone().map(|addr| {
                view! {
                    <div class="evm-invoice-item-address">
                        <Address address=addr />
                    </div>
                }
            })}
        </Card>
    }
}

#[component]
fn InvoiceListSkeleton() -> impl IntoView {
    view! {
        <Stack gap="md">
            <Skeleton height="120px" />
            <Skeleton height="120px" />
            <Skeleton height="120px" />
        </Stack>
    }
}

/// Invoice detail page.
#[component]
pub fn InvoiceDetailPage() -> impl IntoView {
    let params = use_params_map();
    let invoice_id = move || params.get().get("id").unwrap_or_default();

    let invoice = Resource::new(
        move || invoice_id(),
        |id| async move {
            // TODO: Fetch from API
            Ok::<_, String>(Invoice {
                id,
                store_id: "store_001".to_string(),
                amount: "100.00".to_string(),
                currency: "USD".to_string(),
                crypto_amount: Some("0.05".to_string()),
                crypto_currency: Some("ETH".to_string()),
                status: "pending".to_string(),
                payment_address: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f0Ab3D".to_string()),
                chain_id: Some(1),
                created_at: "2024-01-15T10:30:00Z".to_string(),
                expires_at: Some("2024-01-15T11:30:00Z".to_string()),
            })
        },
    );

    view! {
        <div class="evm-page">
            <Suspense fallback=move || view! { <InvoiceDetailSkeleton /> }>
                {move || {
                    invoice.get().map(|result| {
                        match result {
                            Ok(inv) => view! { <InvoiceDetail invoice=inv /> }.into_any(),
                            Err(e) => view! {
                                <Card>
                                    <p class="text-error">"Error loading invoice: " {e}</p>
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
fn InvoiceDetail(invoice: Invoice) -> impl IntoView {
    use ui_kit::components::crypto::{AddressDisplay, QrCodeCard};

    let invoice_id = invoice.id.clone();
    let created_at = invoice.created_at.clone();
    let network = invoice.chain_id
        .and_then(|id| chain_id_to_network(id))
        .unwrap_or(types::Network::Ethereum);

    view! {
        <Stack gap="lg">
            <PageHeader
                title="Invoice Details"
                description=invoice_id.leak()
            />

            <Grid cols=2 gap="lg">
                // Left column - payment info
                <Stack gap="md">
                    <CardWithHeader title="Payment" subtitle="Scan QR code or copy address">
                        <Stack gap="md">
                            {invoice.payment_address.clone().map(|addr| {
                                view! {
                                    <QrCodeCard
                                        data=addr.clone()
                                        label="Payment Address"
                                        size=200
                                    />
                                    <AddressDisplay address=addr label="Address" />
                                }
                            })}
                        </Stack>
                    </CardWithHeader>
                </Stack>

                // Right column - invoice details
                <Stack gap="md">
                    <CardWithHeader title="Invoice Info">
                        <div class="evm-detail-grid">
                            <div class="evm-detail-row">
                                <span class="evm-detail-label">"Status"</span>
                                <StatusBadge status=invoice.status.clone() />
                            </div>
                            <div class="evm-detail-row">
                                <span class="evm-detail-label">"Network"</span>
                                <NetworkBadge network=network />
                            </div>
                            <div class="evm-detail-row">
                                <span class="evm-detail-label">"Amount"</span>
                                <span>{format!("{} {}", invoice.amount, invoice.currency)}</span>
                            </div>
                            {invoice.crypto_amount.clone().map(|amt| {
                                let symbol = invoice.crypto_currency.clone().unwrap_or_default();
                                view! {
                                    <div class="evm-detail-row">
                                        <span class="evm-detail-label">"Crypto Amount"</span>
                                        <span>{format!("{} {}", amt, symbol)}</span>
                                    </div>
                                }
                            })}
                            <div class="evm-detail-row">
                                <span class="evm-detail-label">"Created"</span>
                                <span>{created_at}</span>
                            </div>
                            {invoice.expires_at.clone().map(|exp| {
                                view! {
                                    <div class="evm-detail-row">
                                        <span class="evm-detail-label">"Expires"</span>
                                        <span>{exp}</span>
                                    </div>
                                }
                            })}
                        </div>
                    </CardWithHeader>
                </Stack>
            </Grid>
        </Stack>
    }
}

#[component]
fn InvoiceDetailSkeleton() -> impl IntoView {
    view! {
        <Stack gap="lg">
            <Skeleton height="60px" />
            <Grid cols=2 gap="lg">
                <Skeleton height="400px" />
                <Skeleton height="300px" />
            </Grid>
        </Stack>
    }
}

/// Convert chain ID to Network type.
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

/// Invoice page styles.
pub const INVOICE_STYLES: &str = r#"
.evm-empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--ps-spacing-md);
    padding: var(--ps-spacing-xl);
    text-align: center;
    color: var(--ps-text-muted);
}

.evm-invoice-list {
    display: flex;
    flex-direction: column;
    gap: var(--ps-spacing-md);
}

.evm-invoice-item {
    padding: var(--ps-spacing-md);
}

.evm-invoice-item-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: var(--ps-spacing-sm);
}

.evm-invoice-item-id a {
    font-weight: 600;
    color: var(--ps-primary);
    text-decoration: none;
}

.evm-invoice-item-id a:hover {
    text-decoration: underline;
}

.evm-invoice-item-body {
    display: flex;
    justify-content: space-between;
    align-items: center;
}

.evm-invoice-item-address {
    margin-top: var(--ps-spacing-sm);
    padding-top: var(--ps-spacing-sm);
    border-top: 1px solid var(--ps-border);
}

.evm-detail-grid {
    display: flex;
    flex-direction: column;
    gap: var(--ps-spacing-md);
}

.evm-detail-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
}

.evm-detail-label {
    color: var(--ps-text-muted);
    font-size: var(--ps-font-sm);
}
"#;
