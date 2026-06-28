//! Payment detail page and its inner render component.

use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_params_map;

use crate::api::{EvmApiClient, Payment};
use crate::util::chain_name;

use super::format::{
    format_crypto_amount, format_date, payment_status, payment_status_class, truncate_hash,
};
use super::icons::{IconArrowLeft, IconChevronRight, IconCopy, IconExternalLink, IconInvoice};

/// Payment detail page.
#[component]
pub fn PaymentDetailPage() -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");
    let params = use_params_map();
    let payment_id = move || params.get().get("id").unwrap_or_default();

    let payment_resource = LocalResource::new(move || {
        let api = api.get();
        let id = payment_id();
        async move { api.get_payment(&id).await }
    });

    view! {
        <Suspense fallback=move || view! {
            <div class="loading-container">
                <div class="loading-spinner"></div>
                <p>"Loading payment..."</p>
            </div>
        }>
            {move || payment_resource.get().map(|result| match &*result {
                Err(e) => view! {
                    <div class="error-container">
                        <p class="error-message">{e.to_string()}</p>
                        <A href="/evm/payments" attr:class="btn btn-secondary btn-sm">"Back to payments"</A>
                    </div>
                }.into_any(),
                Ok(payment) => {
                    let payment = payment.clone();
                    view! { <PaymentDetailView payment=payment /> }.into_any()
                }
            })}
        </Suspense>
    }
}

/// Inner component that renders payment detail once data is loaded.
#[component]
fn PaymentDetailView(payment: Payment) -> impl IntoView {
    let network = chain_name(payment.chain_id);
    let status = payment_status(&payment);
    let status_class = payment_status_class(&payment);
    let amount_display = format!(
        "{} {}",
        format_crypto_amount(&payment.amount, payment.decimals),
        payment.asset_symbol
    );
    let detected_display = format_date(&payment.detected_at);
    let confirmed_display = payment
        .confirmed_at
        .as_ref()
        .map(|d| format_date(d))
        .unwrap_or_else(|| "Pending".to_string());
    let from_address = payment
        .from_address
        .clone()
        .unwrap_or_else(|| "Unknown".to_string());
    let block_number = payment
        .block_number
        .map(|b| b.to_string())
        .unwrap_or_else(|| "Pending".to_string());
    let invoice_id = payment.invoice_id.clone();
    let invoice_link = payment.invoice_id.clone();

    // Explorer URL based on chain
    let explorer_base = match payment.chain_id {
        1 => "https://etherscan.io",
        137 => "https://polygonscan.com",
        42161 => "https://arbiscan.io",
        10 => "https://optimistic.etherscan.io",
        8453 => "https://basescan.org",
        56 => "https://bscscan.com",
        43114 => "https://snowtrace.io",
        11155111 => "https://sepolia.etherscan.io",
        _ => "https://etherscan.io",
    };
    let tx_explorer_url = format!("{}/tx/{}", explorer_base, payment.tx_hash);
    let address_explorer_url = format!("{}/address/{}", explorer_base, from_address);

    view! {
        <div class="payment-detail-page">
            // Header
            <div class="payment-detail-header">
                <div class="payment-detail-header-left">
                    <A href="/evm/payments" attr:class="back-link">
                        <IconArrowLeft />
                        "Payments"
                    </A>
                    <div class="payment-detail-title-row">
                        <h1 class="payment-detail-title">{amount_display.clone()}</h1>
                        <span class=status_class>{status}</span>
                    </div>
                    <p class="payment-detail-subtitle">{network}" · "{detected_display.clone()}</p>
                </div>
                <div class="payment-detail-actions">
                    <a href=tx_explorer_url.clone() target="_blank" class="btn btn-secondary btn-sm">
                        <IconExternalLink />
                        "View on Explorer"
                    </a>
                </div>
            </div>

            // Content
            <div class="payment-detail-content">
                // Main Info Cards
                <div class="payment-detail-main">
                    // Transaction Details Card
                    <div class="detail-card">
                        <div class="detail-card-header">
                            <h3>"Transaction Details"</h3>
                        </div>
                        <div class="detail-card-body">
                            <div class="detail-row">
                                <span class="detail-label">"Transaction Hash"</span>
                                <div class="detail-value detail-value-mono">
                                    <span class="tx-hash-full">{payment.tx_hash.clone()}</span>
                                    <button class="btn-icon-xs" title="Copy">
                                        <IconCopy />
                                    </button>
                                </div>
                            </div>
                            <div class="detail-row">
                                <span class="detail-label">"Network"</span>
                                <span class="detail-value">{network}</span>
                            </div>
                            <div class="detail-row">
                                <span class="detail-label">"Block Number"</span>
                                <span class="detail-value">{block_number}</span>
                            </div>
                            <div class="detail-row">
                                <span class="detail-label">"From Address"</span>
                                <div class="detail-value detail-value-mono">
                                    <a href=address_explorer_url target="_blank" class="address-link">
                                        {truncate_hash(&from_address, 10, 8)}
                                    </a>
                                    <button class="btn-icon-xs" title="Copy">
                                        <IconCopy />
                                    </button>
                                </div>
                            </div>
                        </div>
                    </div>

                    // Payment Amount Card
                    <div class="detail-card">
                        <div class="detail-card-header">
                            <h3>"Payment Amount"</h3>
                        </div>
                        <div class="detail-card-body">
                            <div class="detail-row">
                                <span class="detail-label">"Amount Received"</span>
                                <span class="detail-value detail-value-lg">{amount_display}</span>
                            </div>
                            {payment.token_address.clone().map(|addr| view! {
                                <div class="detail-row">
                                    <span class="detail-label">"Token Contract"</span>
                                    <div class="detail-value detail-value-mono">
                                        <span>{truncate_hash(&addr, 10, 8)}</span>
                                        <button class="btn-icon-xs" title="Copy">
                                            <IconCopy />
                                        </button>
                                    </div>
                                </div>
                            })}
                        </div>
                    </div>

                    // Timeline Card
                    <div class="detail-card">
                        <div class="detail-card-header">
                            <h3>"Activity"</h3>
                        </div>
                        <div class="detail-card-body">
                            <div class="timeline">
                                {payment.confirmed_at.is_some().then(|| view! {
                                    <div class="timeline-item timeline-item-success">
                                        <div class="timeline-dot"></div>
                                        <div class="timeline-content">
                                            <span class="timeline-title">"Payment confirmed"</span>
                                            <span class="timeline-desc">"Reached required block confirmations"</span>
                                            <span class="timeline-time">{confirmed_display.clone()}</span>
                                        </div>
                                    </div>
                                })}
                                {(!payment.reorged && payment.confirmed_at.is_none()).then(|| view! {
                                    <div class="timeline-item timeline-item-pending">
                                        <div class="timeline-dot"></div>
                                        <div class="timeline-content">
                                            <span class="timeline-title">"Awaiting confirmation"</span>
                                            <span class="timeline-desc">"Waiting for block confirmations..."</span>
                                        </div>
                                    </div>
                                })}
                                {payment.reorged.then(|| view! {
                                    <div class="timeline-item timeline-item-error">
                                        <div class="timeline-dot"></div>
                                        <div class="timeline-content">
                                            <span class="timeline-title">"Payment invalidated"</span>
                                            <span class="timeline-desc">"Chain reorganization detected"</span>
                                        </div>
                                    </div>
                                })}
                                <div class="timeline-item">
                                    <div class="timeline-dot"></div>
                                    <div class="timeline-content">
                                        <span class="timeline-title">"Payment detected"</span>
                                        <span class="timeline-desc">"Transaction seen on chain"</span>
                                        <span class="timeline-time">{detected_display}</span>
                                    </div>
                                </div>
                            </div>
                        </div>
                    </div>
                </div>

                // Sidebar
                <div class="payment-detail-sidebar">
                    <div class="detail-card">
                        <div class="detail-card-header">
                            <h3>"Related Invoice"</h3>
                        </div>
                        <div class="detail-card-body">
                            <A href=format!("/evm/invoices/{}", invoice_link) attr:class="related-invoice-link">
                                <div class="related-invoice-info">
                                    <IconInvoice />
                                    <span>{truncate_hash(&invoice_id, 8, 4)}</span>
                                </div>
                                <IconChevronRight />
                            </A>
                        </div>
                    </div>

                    <div class="detail-card">
                        <div class="detail-card-header">
                            <h3>"Status"</h3>
                        </div>
                        <div class="detail-card-body">
                            <div class="status-info">
                                <span class=format!("{} status-badge-lg", status_class)>{status}</span>
                                {payment.reorged.then(|| view! {
                                    <p class="status-warning">
                                        "This payment was invalidated due to a chain reorganization."
                                    </p>
                                })}
                                {(!payment.reorged && payment.confirmed_at.is_none()).then(|| view! {
                                    <p class="status-info-text">
                                        "Waiting for block confirmations..."
                                    </p>
                                })}
                            </div>
                        </div>
                    </div>

                    <div class="detail-card">
                        <div class="detail-card-header">
                            <h3>"Raw Data"</h3>
                        </div>
                        <div class="detail-card-body">
                            <pre class="metadata-json">{format!(r#"{{
  "id": "{}",
  "chain_id": {},
  "invoice_id": "{}",
  "tx_hash": "{}",
  "amount": "{}",
  "asset_symbol": "{}",
  "block_number": {}
}}"#,
                                payment.id,
                                payment.chain_id,
                                payment.invoice_id,
                                payment.tx_hash,
                                payment.amount,
                                payment.asset_symbol,
                                payment.block_number.map(|b| b.to_string()).unwrap_or_else(|| "null".to_string())
                            )}</pre>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    }
}
