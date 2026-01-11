//! Payments page - view payment history.

use leptos::prelude::*;
use ui_kit::components::{Card, PageHeader, Skeleton, Stack};
use ui_kit::components::crypto::{Address, CryptoAmount, NetworkBadge, StatusBadge};

use crate::api::{AssetType, Payment};

/// Helper to determine payment status display.
fn payment_status_display(payment: &Payment) -> &'static str {
    if payment.reorged {
        "reorged"
    } else if payment.confirmed_at.is_some() {
        "confirmed"
    } else {
        "pending"
    }
}

/// Payments list page.
#[component]
pub fn PaymentsPage() -> impl IntoView {
    let payments = Resource::new(|| (), |_| async {
        // TODO: Fetch from API
        Ok::<_, String>(vec![
            Payment {
                id: "pay_001".to_string(),
                invoice_id: "inv_001".to_string(),
                chain_id: 1,
                asset_type: AssetType::Native,
                amount: "50000000000000000".to_string(), // 0.05 ETH in wei
                asset_symbol: "ETH".to_string(),
                token_address: None,
                tx_hash: "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef".to_string(),
                block_number: Some(19234500),
                detected_at: "2024-01-15T10:35:00Z".to_string(),
                confirmed_at: Some("2024-01-15T10:40:00Z".to_string()),
                from_address: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f0Ab3D".to_string()),
                reorged: false,
                credited_amount: Some("100.00".to_string()),
                rate_used: Some("0.0005".to_string()),
            },
        ])
    });

    view! {
        <div class="evm-page">
            <PageHeader
                title="Payments"
                description="View your payment history"
            />

            <Suspense fallback=move || view! { <PaymentListSkeleton /> }>
                {move || {
                    payments.get().map(|result| {
                        match result {
                            Ok(list) => view! { <PaymentList payments=list /> }.into_any(),
                            Err(e) => view! {
                                <Card>
                                    <p class="text-error">"Error loading payments: " {e}</p>
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
fn PaymentList(payments: Vec<Payment>) -> impl IntoView {
    if payments.is_empty() {
        return view! {
            <Card>
                <div class="evm-empty-state">
                    <p>"No payments received yet"</p>
                </div>
            </Card>
        }.into_any();
    }

    view! {
        <div class="evm-payment-list">
            {payments.into_iter().map(|payment| {
                view! { <PaymentListItem payment=payment /> }
            }).collect_view()}
        </div>
    }.into_any()
}

#[component]
fn PaymentListItem(payment: Payment) -> impl IntoView {
    let network = chain_id_to_network(payment.chain_id)
        .unwrap_or(types::Network::Ethereum);
    let invoice_id = payment.invoice_id.clone();
    let invoice_id_link = payment.invoice_id.clone();
    let detected_at = payment.detected_at.clone();
    let status = payment_status_display(&payment).to_string();
    let from_address = payment.from_address.clone().unwrap_or_else(|| "Unknown".to_string());

    view! {
        <Card class="evm-payment-item">
            <div class="evm-payment-item-header">
                <div class="evm-payment-item-amount">
                    <CryptoAmount
                        amount=payment.amount.clone()
                        symbol=payment.asset_symbol.clone()
                        size="lg"
                    />
                </div>
                <StatusBadge status=status />
            </div>

            <div class="evm-payment-item-details">
                <div class="evm-payment-detail">
                    <span class="evm-payment-detail-label">"Network"</span>
                    <NetworkBadge network=network />
                </div>
                <div class="evm-payment-detail">
                    <span class="evm-payment-detail-label">"From"</span>
                    <Address address=from_address />
                </div>
                <div class="evm-payment-detail">
                    <span class="evm-payment-detail-label">"Tx Hash"</span>
                    <Address address=payment.tx_hash.clone() prefix_len=10 suffix_len=8 />
                </div>
                {payment.credited_amount.clone().map(|credited| view! {
                    <div class="evm-payment-detail">
                        <span class="evm-payment-detail-label">"Credited"</span>
                        <span class="evm-payment-credited">"$"{credited}</span>
                    </div>
                })}
            </div>

            <div class="evm-payment-item-footer">
                <span class="evm-payment-invoice">
                    "Invoice: "
                    <a href=format!("/evm/invoices/{}", invoice_id_link)>{invoice_id}</a>
                </span>
                <span class="evm-payment-time">{detected_at}</span>
            </div>
        </Card>
    }
}

#[component]
fn PaymentListSkeleton() -> impl IntoView {
    view! {
        <Stack gap="md">
            <Skeleton height="150px" />
            <Skeleton height="150px" />
            <Skeleton height="150px" />
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

/// Payment page styles.
pub const PAYMENT_STYLES: &str = r#"
.evm-payment-list {
    display: flex;
    flex-direction: column;
    gap: var(--ps-spacing-md);
}

.evm-payment-item {
    padding: var(--ps-spacing-md);
}

.evm-payment-item-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: var(--ps-spacing-md);
}

.evm-payment-item-details {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: var(--ps-spacing-sm);
    padding: var(--ps-spacing-md) 0;
    border-top: 1px solid var(--ps-border);
    border-bottom: 1px solid var(--ps-border);
}

.evm-payment-detail {
    display: flex;
    flex-direction: column;
    gap: var(--ps-spacing-xs);
}

.evm-payment-detail-label {
    font-size: var(--ps-font-sm);
    color: var(--ps-text-muted);
}

.evm-payment-confirmations {
    font-weight: 600;
    color: var(--ps-success);
}

.evm-payment-item-footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-top: var(--ps-spacing-md);
    font-size: var(--ps-font-sm);
}

.evm-payment-invoice a {
    color: var(--ps-primary);
    text-decoration: none;
}

.evm-payment-invoice a:hover {
    text-decoration: underline;
}

.evm-payment-time {
    color: var(--ps-text-muted);
}
"#;
