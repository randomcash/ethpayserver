use axum::{Json, extract::State, http::StatusCode};
use chrono::Utc;

use ::types::{InvoiceId, InvoiceStatus, StoreId, traits::InvoiceData};
use auth::{SessionService, repository::UserStoreRepository};
use data_service::StorePaymentMethodReader;
use rust_decimal::Decimal;

use crate::api::extractors::AuthenticatedUser;
use crate::metrics;
use crate::state::PgAppState;
use ::types::currency::DEFAULT_INVOICE_EXPIRATION_SECS;
use rates::{RateError, is_fiat_currency};

use super::{
    CreateInvoiceRequest, InvoiceResponse, apply_token_policy_filter,
    convert_human_to_smallest_unit, convert_to_crypto_smallest_unit, extract_customer_email,
    invoice_error, rate_stale_reject_secs, rate_stale_warn_secs,
};

/// Create a new invoice.
///
/// Requires `cancreateinvoice` permission on the store.
/// The invoice is network-agnostic - payment methods are determined by store configuration.
#[utoipa::path(
    post,
    path = "/invoices",
    tag = "invoices",
    security(("bearer_auth" = [])),
    request_body = CreateInvoiceRequest,
    responses(
        (status = 201, description = "Invoice created", body = InvoiceResponse),
        (status = 400, description = "Invalid request or no payment methods configured"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
#[allow(clippy::too_many_lines, clippy::cognitive_complexity)] // validation + payment-option setup is one logical flow
pub async fn create_invoice<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Json(req): Json<CreateInvoiceRequest>,
) -> Result<(StatusCode, Json<InvoiceResponse>), (StatusCode, Json<serde_json::Value>)>
where
    A: SessionService + 'static,
{
    // Check permission on the store
    let has_permission = state
        .data_service
        .user_has_store_permission(
            user.id,
            StoreId(req.store_id),
            "ethpay.store.cancreateinvoice",
        )
        .await
        .map_err(|_| {
            invoice_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Failed to check store permissions",
            )
        })?;

    if !has_permission {
        return Err(invoice_error(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Insufficient permissions to create invoices for this store",
        ));
    }

    // Get ALL enabled payment methods for the store
    let mut payment_methods =
        StorePaymentMethodReader::get_enabled_payment_methods(&*state.data_service, req.store_id)
            .await
            .map_err(|_| {
                invoice_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "Failed to load payment methods",
                )
            })?;

    // Apply token policy filter (allowlist/blocklist)
    if let Some(policy) =
        data_service::StoreTokenPolicyReader::get_token_policy(&*state.data_service, req.store_id)
            .await
            .map_err(|_| {
                invoice_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "Failed to load token policy",
                )
            })?
    {
        apply_token_policy_filter(&mut payment_methods, &policy);
    }

    if payment_methods.is_empty() {
        tracing::warn!(
            "Store {} has no enabled payment methods (after token policy filter)",
            req.store_id
        );
        return Err(invoice_error(
            StatusCode::BAD_REQUEST,
            "no_payment_methods",
            "Store has no enabled payment methods. Configure at least one payment method before creating invoices.",
        ));
    }

    // Pre-validate: Fetch rates for cross-currency invoices to avoid creating orphan invoices
    // For same-asset invoices (e.g., ETH invoice paid with ETH), no rate needed
    // Stores (payment_method_index, crypto_amount, rate_string, rate_timestamp)
    let mut validated_methods: Vec<(
        usize,
        String,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
    )> = Vec::new();
    let invoice_currency_upper = req.currency.to_uppercase();

    for (idx, payment_method) in payment_methods.iter().enumerate() {
        let asset_symbol_upper = payment_method.asset_symbol.to_uppercase();

        // Check if this is a same-asset payment (no conversion needed)
        if invoice_currency_upper == asset_symbol_upper {
            // Same asset: use amount directly, convert to smallest units if needed
            let amount = if is_fiat_currency(&req.currency) {
                // Fiat currency can't be same as crypto asset, this shouldn't happen
                tracing::warn!(
                    currency = %req.currency,
                    asset = %payment_method.asset_symbol,
                    "Fiat currency matched asset symbol, skipping"
                );
                continue;
            } else {
                // Crypto-denominated invoice with matching asset
                // Amount is in human-readable format, convert to smallest units
                convert_human_to_smallest_unit(&req.amount, payment_method.decimals).map_err(
                    |e| {
                        tracing::error!(
                            currency = %req.currency,
                            amount = %req.amount,
                            error = %e,
                            "Failed to convert amount to smallest units"
                        );
                        invoice_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_amount",
                            "Invalid amount format",
                        )
                    },
                )?
            };

            tracing::debug!(
                currency = %req.currency,
                asset = %payment_method.asset_symbol,
                amount = %amount,
                "Same-asset payment, no rate conversion needed"
            );

            validated_methods.push((idx, amount, None, None));
        } else {
            // Cross-currency: need to fetch exchange rate
            match state
                .rate_provider
                .get_rate(&req.currency, &payment_method.asset_symbol)
                .await
            {
                Ok(exchange_rate) => {
                    // Validate rate is positive
                    if exchange_rate.rate <= Decimal::ZERO {
                        tracing::error!(
                            from = %req.currency,
                            to = %payment_method.asset_symbol,
                            rate = %exchange_rate.rate,
                            "Received non-positive exchange rate"
                        );
                        return Err(invoice_error(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "invalid_rate",
                            "Received invalid exchange rate, please try again later",
                        ));
                    }

                    // Staleness guard: reject rates older than threshold
                    let reject_secs = rate_stale_reject_secs();
                    if exchange_rate.is_stale(chrono::Duration::seconds(reject_secs)) {
                        tracing::error!(
                            from = %req.currency,
                            to = %payment_method.asset_symbol,
                            timestamp = %exchange_rate.timestamp,
                            max_age_secs = reject_secs,
                            "Exchange rate too stale, rejecting"
                        );
                        return Err(invoice_error(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "rate_stale",
                            "Exchange rate data is too old, please try again later",
                        ));
                    }

                    // Warn for rates older than warning threshold
                    let warn_secs = rate_stale_warn_secs();
                    if exchange_rate.is_stale(chrono::Duration::seconds(warn_secs)) {
                        tracing::warn!(
                            from = %req.currency,
                            to = %payment_method.asset_symbol,
                            timestamp = %exchange_rate.timestamp,
                            warn_age_secs = warn_secs,
                            "Exchange rate may be slightly outdated"
                        );
                    }

                    // Convert invoice currency to crypto smallest units
                    let amount = convert_to_crypto_smallest_unit(
                        &req.amount,
                        exchange_rate.rate,
                        payment_method.decimals,
                    )
                    .map_err(|e| {
                        tracing::error!(
                            currency = %req.currency,
                            amount = %req.amount,
                            error = %e,
                            "Failed to convert amount to crypto"
                        );
                        invoice_error(
                            StatusCode::BAD_REQUEST,
                            "conversion_error",
                            "Invalid amount or conversion error",
                        )
                    })?;

                    tracing::debug!(
                        from = %req.currency,
                        to = %payment_method.asset_symbol,
                        rate = %exchange_rate.rate,
                        invoice_amount = %req.amount,
                        crypto_amount = %amount,
                        "Converted invoice currency to crypto"
                    );

                    validated_methods.push((
                        idx,
                        amount,
                        Some(exchange_rate.rate.to_string()),
                        Some(exchange_rate.timestamp),
                    ));
                }
                Err(RateError::UnsupportedPair { from, to }) => {
                    tracing::warn!(
                        from = %from,
                        to = %to,
                        "Unsupported rate pair, skipping payment method"
                    );
                    // Skip this payment method - don't add to validated_methods
                }
                Err(e) => {
                    tracing::error!(
                        currency = %req.currency,
                        asset = %payment_method.asset_symbol,
                        error = %e,
                        "Failed to fetch exchange rate"
                    );
                    return Err(invoice_error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "rate_unavailable",
                        "Exchange rate service is unavailable, please try again later",
                    ));
                }
            }
        }
    }

    // Check if any payment methods are valid BEFORE creating invoice
    if validated_methods.is_empty() {
        tracing::error!(
            store_id = %req.store_id,
            currency = %req.currency,
            "No payment methods with supported rate pairs for this currency"
        );
        return Err(invoice_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "no_supported_pairs",
            "No payment methods support the requested invoice currency",
        ));
    }

    let expiration_secs = req
        .expiration_seconds
        .unwrap_or(DEFAULT_INVOICE_EXPIRATION_SECS);
    let expires_at = Utc::now() + chrono::Duration::seconds(expiration_secs as i64);

    // Merge customer_email into metadata so the generated DB column picks it up.
    let metadata = match (req.customer_email, req.metadata) {
        (Some(email), Some(mut meta)) => {
            if let Some(obj) = meta.as_object_mut() {
                obj.entry("customer_email")
                    .or_insert_with(|| serde_json::Value::String(email));
            }
            Some(meta)
        }
        (Some(email), None) => Some(serde_json::json!({ "customer_email": email })),
        (None, meta) => meta,
    };

    // Create invoice (network-agnostic) - only after validating payment methods
    let invoice = InvoiceData {
        id: InvoiceId::new(),
        store_id: StoreId(req.store_id),
        currency: req.currency.clone(),
        status: InvoiceStatus::Pending,
        amount: req.amount.clone(),
        amount_received: "0".to_string(),
        created_at: Utc::now(),
        expires_at,
        metadata,
        extra: None,
    };

    ::types::InvoiceWriter::upsert(&*state.data_service, &invoice)
        .await
        .map_err(|_| {
            invoice_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Failed to create invoice",
            )
        })?;

    // Create payment options for each validated payment method
    let created_options = super::payment_options::build_payment_options(
        &state,
        &invoice,
        &payment_methods,
        validated_methods,
    )
    .await?;

    // Defensive check: should never happen since we pre-validate payment methods
    debug_assert!(
        !created_options.is_empty(),
        "Pre-validation should ensure at least one valid method"
    );

    // Record metrics
    metrics::record_invoice_created(&invoice.currency);

    let customer_email = extract_customer_email(&invoice.metadata);
    let response = InvoiceResponse {
        id: invoice.id.0,
        currency: invoice.currency,
        status: invoice.status.to_string(),
        amount: invoice.amount,
        amount_received: invoice.amount_received,
        created_at: invoice.created_at,
        expires_at: invoice.expires_at,
        metadata: invoice.metadata,
        customer_email,
        payment_options: created_options.into_iter().map(|o| o.into()).collect(),
    };

    Ok((StatusCode::CREATED, Json(response)))
}
