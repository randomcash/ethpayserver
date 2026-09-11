use axum::{Json, http::StatusCode};
use chrono::Utc;
use uuid::Uuid;

use ::types::{
    DerivationAllocation, PaymentMethodId, PaymentOptionData, PaymentOptionId,
    StorePaymentMethodWriter, traits::InvoiceData,
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

/// Derive a payment option for each validated payment method, writing nothing.
///
/// Every option is built in memory so the whole set can be written in one
/// transaction by the caller. This used to persist each option and its watched
/// address as it went, which meant a failure on the third method left an
/// invoice committed with two - payable in some assets and not others, and
/// quoting a customer an address nobody was watching.
///
/// The one thing that *is* written here is the derivation counter, and that is
/// deliberate: `allocate_derivation` advances it before an address exists, and
/// the advance must stand even if everything after rolls back. Burning an index
/// costs nothing. Returning one risks issuing the same address twice.
///
/// Returns the options in input order, each with what the monitor needs to be
/// told once the invoice is committed.
pub(crate) async fn derive_payment_options<A: SessionService>(
    state: &PgAppState<A>,
    invoice: &InvoiceData,
    payment_methods: &[data_service::StorePaymentMethod],
    validated_methods: Vec<ValidatedMethod>,
) -> Result<Vec<DerivedOption>, (StatusCode, Json<serde_json::Value>)> {
    let mut derived: Vec<DerivedOption> = Vec::with_capacity(validated_methods.len());

    for (method_idx, crypto_amount, rate_str, rate_at) in validated_methods {
        derived.push(
            derive_one_payment_option(
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

    Ok(derived)
}

/// A payment option that exists in memory but not yet in the database, together
/// with the parsed values the monitor notification needs.
///
/// The address is carried in its parsed form rather than re-parsed from the
/// option later: it was already parsed to build the option, and a second parse
/// is a second chance to disagree.
pub(crate) struct DerivedOption {
    pub option: PaymentOptionData,
    pub address: Address,
    pub expected_amount: Option<U256>,
    pub token_contract: Option<Address>,
}

/// Tell the monitor about every address on a committed invoice.
///
/// Best-effort, and deliberately after the commit. Notifying earlier announced
/// addresses for an invoice that might still fail to be written - the monitor
/// would then be watching for money against a payment option that does not
/// exist. The retry service covers a missed notification; it cannot unsend one.
pub(crate) async fn notify_monitor_for<A: SessionService>(
    state: &PgAppState<A>,
    invoice: &InvoiceData,
    derived: &[DerivedOption],
) {
    for d in derived {
        notify_evm_watch(
            state,
            &invoice.id.0,
            &d.option.payment_address,
            &d.option.chain_id,
            d.option.token_address.as_deref(),
            d.address,
            d.expected_amount,
            d.token_contract,
        )
        .await;
    }
}

/// Derive an address for one payment method and build its option in memory.
///
/// Persists nothing but the derivation counter - see `derive_payment_options`.
async fn derive_one_payment_option<A: SessionService>(
    state: &PgAppState<A>,
    invoice: &InvoiceData,
    payment_method: &data_service::StorePaymentMethod,
    crypto_amount: String,
    rate_str: Option<String>,
    rate_at: Option<chrono::DateTime<Utc>>,
) -> Result<DerivedOption, (StatusCode, Json<serde_json::Value>)> {
    let (address, allocation) = derive_payment_address(state, payment_method).await?;

    let option = PaymentOptionData {
        id: PaymentOptionId(Uuid::new_v4()),
        invoice_id: invoice.id.clone(),
        payment_method_id: PaymentMethodId::new(
            &payment_method.asset_symbol,
            &payment_method.chain_id,
        ),
        chain_id: payment_method.chain_id.clone(),
        asset_symbol: payment_method.asset_symbol.clone(),
        token_address: payment_method.token_address.clone(),
        decimals: payment_method.decimals,
        payment_address: address.to_string(),
        // Record which key produced this address and at what index. The address
        // alone no longer implies a wallet now that stores can share one.
        wallet_id: Some(allocation.wallet_id),
        derivation_index: Some(allocation.index),
        amount: crypto_amount,
        rate: rate_str,
        rate_at,
        is_active: true,
        created_at: Utc::now(),
    };

    let expected_amount = option.amount.parse::<U256>().ok();
    let token_contract: Option<Address> = payment_method
        .token_address
        .as_ref()
        .and_then(|addr| addr.parse().ok());

    Ok(DerivedOption {
        option,
        address,
        expected_amount,
        token_contract,
    })
}

/// Allocate the next derivation index for a payment method and derive its
/// address from the stored xpub.
///
/// Kept separate from `derive_one_payment_option` so index allocation and key
/// derivation — the two steps that must not silently reuse an address — read
/// as one unit.
///
/// Returns the address and the allocation it came from; both the wallet and
/// the index are recorded on the payment option so the pairing can be audited
/// later.
async fn derive_payment_address<A: SessionService>(
    state: &PgAppState<A>,
    payment_method: &data_service::StorePaymentMethod,
) -> Result<(Address, DerivationAllocation), (StatusCode, Json<serde_json::Value>)> {
    // One call takes the index and returns the key it was taken from. The
    // xpub on `payment_method` is deliberately NOT used here: it was read
    // earlier, and a rotation or an override change committing since would
    // make it a different wallet's key than the one whose counter just moved -
    // deriving from the pair would burn an index on one wallet while handing
    // out an address the other will issue again later.
    let allocation =
        StorePaymentMethodWriter::allocate_derivation(&*state.data_service, payment_method.id)
            .await
            .map_err(|_| {
                invoice_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "Failed to allocate payment address",
                )
            })?;

    let deriver = XpubDeriver::from_xpub(&allocation.xpub).map_err(|_| {
        invoice_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "Failed to derive payment address",
        )
    })?;
    let address = deriver
        .derive_address(allocation.index as u32)
        .map_err(|_| {
            invoice_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Failed to derive payment address",
            )
        })?;

    Ok((address, allocation))
}
