use axum::{Json, http::StatusCode};
use chrono::Utc;
use uuid::Uuid;

use ::types::{
    PaymentMethodId, PaymentOptionData, PaymentOptionId, StorePaymentMethodWriter,
    WatchedAddressWriter, traits::InvoiceData,
};
use auth::SessionService;
use evm::{Address, U256, XpubDeriver};

use crate::state::PgAppState;

use super::{invoice_error, notify_evm_watch};

/// A payment method validated for invoice creation, with its computed crypto amount and rate.
/// Tuple layout: `(payment_method_index, crypto_amount, rate_string, rate_timestamp)`.
pub(crate) type ValidatedMethod = (
    usize,
    String,
    Option<String>,
    Option<chrono::DateTime<chrono::Utc>>,
);

/// Build and persist a payment option for each validated payment method.
///
/// For every method this derives a fresh payment address from the method's xpub,
/// persists the `PaymentOptionData` and its watched address, then best-effort notifies
/// the EVM monitor. Returns the created options in input order.
///
/// Extracted from `create_invoice` (RCS-176); behavior is unchanged.
pub(crate) async fn build_payment_options<A: SessionService>(
    state: &PgAppState<A>,
    invoice: &InvoiceData,
    payment_methods: &[data_service::StorePaymentMethod],
    validated_methods: Vec<ValidatedMethod>,
) -> Result<Vec<PaymentOptionData>, (StatusCode, Json<serde_json::Value>)> {
    let mut created_options: Vec<PaymentOptionData> = Vec::with_capacity(validated_methods.len());

    for (method_idx, crypto_amount, rate_str, rate_at) in validated_methods {
        created_options.push(
            build_one_payment_option(
                state,
                invoice,
                &payment_methods[method_idx],
                crypto_amount,
                rate_str,
                rate_at,
            )
            .await?,
        );
    }

    Ok(created_options)
}

/// Derive an address for one payment method, persist the option, register the
/// watched address and notify the monitor.
///
/// Split out of `build_payment_options` so the outer function is just the loop.
/// The per-method work is six sequential fallible steps and ran to 100 lines
/// inline, over the 80-line lint that this ticket exists to satisfy.
async fn build_one_payment_option<A: SessionService>(
    state: &PgAppState<A>,
    invoice: &InvoiceData,
    payment_method: &data_service::StorePaymentMethod,
    crypto_amount: String,
    rate_str: Option<String>,
    rate_at: Option<chrono::DateTime<Utc>>,
) -> Result<PaymentOptionData, (StatusCode, Json<serde_json::Value>)> {
    let address = derive_payment_address(state, payment_method).await?;
    let payment_address = address.to_string();

    // Create payment option with calculated amount and rate
    let payment_option = PaymentOptionData {
        id: PaymentOptionId(Uuid::new_v4()),
        invoice_id: invoice.id.clone(),
        payment_method_id: PaymentMethodId::new(
            &payment_method.asset_symbol,
            payment_method.chain_id,
        ),
        chain_id: payment_method.chain_id,
        asset_symbol: payment_method.asset_symbol.clone(),
        token_address: payment_method.token_address.clone(),
        decimals: payment_method.decimals,
        payment_address: payment_address.clone(),
        amount: crypto_amount,
        rate: rate_str,
        rate_at,
        is_active: true,
        created_at: Utc::now(),
    };

    data_service::PaymentOptionWriter::create(&*state.data_service, &payment_option)
        .await
        .map_err(|_| {
            invoice_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Failed to create payment option",
            )
        })?;

    // Save watched address to database (required for payment detection & retry mechanism)
    let token_address_str = payment_method.token_address.as_deref();
    WatchedAddressWriter::upsert(
        &*state.data_service,
        &payment_address,
        &payment_option.id,
        payment_method.chain_id,
        token_address_str,
    )
    .await
    .map_err(|e| {
        tracing::error!(
            address = %payment_address,
            invoice_id = %invoice.id.0,
            error = %e,
            "Failed to save watched address - invoice creation aborted"
        );
        invoice_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "Failed to save watched address",
        )
    })?;

    // Notify the EVM monitor (best-effort — retry service handles misses)
    let expected_amount = payment_option.amount.parse::<U256>().ok();
    let token_contract: Option<Address> = payment_method
        .token_address
        .as_ref()
        .and_then(|addr| addr.parse().ok());
    notify_evm_watch(
        state,
        &invoice.id.0,
        &payment_address,
        payment_method.chain_id,
        token_address_str,
        address,
        expected_amount,
        token_contract,
    )
    .await;

    Ok(payment_option)
}

/// Allocate the next derivation index for a payment method and derive its
/// address from the stored xpub.
///
/// Kept separate from `build_one_payment_option` so index allocation and key
/// derivation — the two steps that must not silently reuse an address — read
/// as one unit.
async fn derive_payment_address<A: SessionService>(
    state: &PgAppState<A>,
    payment_method: &data_service::StorePaymentMethod,
) -> Result<Address, (StatusCode, Json<serde_json::Value>)> {
    // Get and increment derivation index for this payment method
    let index =
        StorePaymentMethodWriter::next_derivation_index(&*state.data_service, payment_method.id)
            .await
            .map_err(|_| {
                invoice_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "Failed to allocate payment address",
                )
            })?;

    // Derive payment address from the payment method's xpub
    let deriver = XpubDeriver::from_xpub(&payment_method.xpub).map_err(|_| {
        invoice_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "Failed to derive payment address",
        )
    })?;
    let address = deriver.derive_address(index as u32).map_err(|_| {
        invoice_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "Failed to derive payment address",
        )
    })?;

    Ok(address)
}
