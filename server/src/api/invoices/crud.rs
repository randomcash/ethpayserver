use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::Utc;
use uuid::Uuid;

use ::types::{
    InvoiceId, InvoiceQueryParams, InvoiceReader, InvoiceStatus, InvoiceWriter, PaymentMethodId,
    PaymentOptionData, PaymentOptionId, StoreId, StorePaymentMethodWriter, WatchedAddressWriter,
    traits::InvoiceData,
};
use auth::{SessionService, repository::UserStoreRepository};
use data_service::{PaymentOptionReader, PaymentOptionWriter, StorePaymentMethodReader};
use evm::{Address, U256, XpubDeriver};
use rust_decimal::Decimal;

use super::super::extractors::{AdminAuth, AuthenticatedUser};
use super::{
    CreateInvoiceRequest, InvoiceListResponse, InvoiceResponse, ListInvoicesQuery,
    apply_token_policy_filter, convert_human_to_smallest_unit, convert_to_crypto_smallest_unit,
    extract_customer_email, invoice_error, rate_stale_reject_secs, rate_stale_warn_secs,
};
use crate::metrics;
use crate::services::EVMMonitor;
use crate::state::PgAppState;
use ::types::currency::DEFAULT_INVOICE_EXPIRATION_SECS;
use rates::{RateError, is_fiat_currency};

/// List invoices with optional filters.
///
/// Users can only see invoices for stores they are members of.
/// store_id is required unless user is a server admin.
#[utoipa::path(
    get,
    path = "/invoices",
    tag = "invoices",
    security(("bearer_auth" = [])),
    params(ListInvoicesQuery),
    responses(
        (status = 200, description = "List of invoices", body = InvoiceListResponse),
        (status = 400, description = "store_id required"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Not a member of the store"),
    )
)]
pub async fn list_invoices<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Query(query): Query<ListInvoicesQuery>,
) -> Result<Json<InvoiceListResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    // For non-admins, store_id is required
    let store_id = match query.store_id {
        Some(id) => id,
        None => {
            // Admins can list all invoices, non-admins need store_id
            if user.role != auth::Role::ServerAdmin {
                return Err(StatusCode::BAD_REQUEST);
            }
            // For admins without store_id, we'll query all
            uuid::Uuid::nil()
        }
    };

    // Check user has access to the store (unless admin or no store filter)
    if store_id != uuid::Uuid::nil() {
        let is_member = state
            .data_service
            .get_user_store(user.id, StoreId(store_id))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .is_some();

        if !is_member && user.role != auth::Role::ServerAdmin {
            return Err(StatusCode::FORBIDDEN);
        }
    }

    let mut params = InvoiceQueryParams::new();

    if let Some(status) = query.status {
        let status: InvoiceStatus = status.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
        params = params.with_status(status);
    }

    if let Some(currency) = query.currency {
        params = params.with_currency(currency);
    }

    if let Some(limit) = query.limit {
        params = params.with_limit(limit);
    }

    if let Some(offset) = query.offset {
        params = params.with_offset(offset);
    }

    // Add store_id filter if provided (nil means admin querying all)
    if store_id != uuid::Uuid::nil() {
        params = params.with_store_id(StoreId(store_id));
    }

    let (total, invoices) = InvoiceReader::query(&*state.data_service, &params)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Get payment options for each invoice
    let mut responses = Vec::with_capacity(invoices.len());
    for invoice in invoices {
        let options = PaymentOptionReader::get_for_invoice(&*state.data_service, &invoice.id)
            .await
            .unwrap_or_default();

        let customer_email = extract_customer_email(&invoice.metadata);
        responses.push(InvoiceResponse {
            id: invoice.id.0,
            currency: invoice.currency,
            status: invoice.status.to_string(),
            amount: invoice.amount,
            amount_received: invoice.amount_received,
            created_at: invoice.created_at,
            expires_at: invoice.expires_at,
            metadata: invoice.metadata,
            customer_email,
            payment_options: options.into_iter().map(Into::into).collect(),
        });
    }

    Ok(Json(InvoiceListResponse {
        total,
        invoices: responses,
    }))
}

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
                        invoice_error(StatusCode::BAD_REQUEST, "invalid_amount", e)
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
                        invoice_error(StatusCode::BAD_REQUEST, "conversion_error", e)
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

    InvoiceWriter::upsert(&*state.data_service, &invoice)
        .await
        .map_err(|_| {
            invoice_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Failed to create invoice",
            )
        })?;

    // Create payment options for each validated payment method
    let mut created_options: Vec<PaymentOptionData> = Vec::with_capacity(validated_methods.len());

    for (method_idx, crypto_amount, rate_str, rate_at) in validated_methods {
        let payment_method = &payment_methods[method_idx];

        // Get and increment derivation index for this payment method
        let index = StorePaymentMethodWriter::next_derivation_index(
            &*state.data_service,
            payment_method.id,
        )
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

        PaymentOptionWriter::create(&*state.data_service, &payment_option)
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

        // Send WatchAddress command to EVM monitor if configured
        if let Some(ref monitor) = state.evm_monitor {
            // Parse invoice ID as UUID
            let invoice_uuid = Uuid::parse_str(&invoice.id.0).map_err(|e| {
                tracing::error!("Failed to parse invoice ID as UUID: {}", e);
                invoice_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "Internal error processing invoice",
                )
            })?;

            // Parse amount as U256 (optional - only if we can parse it)
            let expected_amount = payment_option.amount.parse::<U256>().ok();

            // Token contract address (for ERC20 payments)
            let token_contract: Option<Address> = payment_method
                .token_address
                .as_ref()
                .and_then(|addr| addr.parse().ok());

            match monitor
                .watch_address_by_chain_id(
                    payment_method.chain_id,
                    address,
                    invoice_uuid,
                    expected_amount,
                    token_contract,
                )
                .await
            {
                Ok(()) => {
                    // Mark as notified in database
                    if let Err(e) = WatchedAddressWriter::mark_notified(
                        &*state.data_service,
                        &payment_address,
                        payment_method.chain_id,
                        token_address_str,
                    )
                    .await
                    {
                        tracing::warn!(
                            address = %payment_address,
                            error = %e,
                            "Failed to mark watch as notified"
                        );
                    }
                }
                Err(e) => {
                    // Log the error but don't fail invoice creation
                    // The address is recorded in the database and retry service will handle it
                    tracing::warn!(
                        invoice_id = %invoice_uuid,
                        address = %address,
                        error = %e,
                        "Failed to send WatchAddress command, will be retried"
                    );
                }
            }
        }

        created_options.push(payment_option);
    }

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

/// Get an invoice by ID.
///
/// User must be a member of the store the invoice belongs to.
#[utoipa::path(
    get,
    path = "/invoices/{invoice_id}",
    tag = "invoices",
    security(("bearer_auth" = [])),
    params(
        ("invoice_id" = String, Path, description = "Invoice ID")
    ),
    responses(
        (status = 200, description = "Invoice details", body = InvoiceResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Not a member of the invoice's store"),
        (status = 404, description = "Invoice not found"),
    )
)]
pub async fn get_invoice<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(invoice_id): Path<String>,
) -> Result<Json<InvoiceResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let id = InvoiceId::from_string(invoice_id);

    let invoice = InvoiceReader::get(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Check user has access to the invoice's store (unless admin)
    if user.role != auth::Role::ServerAdmin {
        let is_member = state
            .data_service
            .get_user_store(user.id, invoice.store_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .is_some();

        if !is_member {
            return Err(StatusCode::FORBIDDEN);
        }
    }

    let options = PaymentOptionReader::get_for_invoice(&*state.data_service, &id)
        .await
        .unwrap_or_default();

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
        payment_options: options.into_iter().map(Into::into).collect(),
    };

    Ok(Json(response))
}

/// Cancel an invoice (admin only).
///
/// Only works for pending/processing invoices.
#[utoipa::path(
    post,
    path = "/admin/invoices/{invoice_id}/cancel",
    tag = "admin",
    security(("bearer_auth" = [])),
    params(
        ("invoice_id" = String, Path, description = "Invoice ID")
    ),
    responses(
        (status = 200, description = "Invoice cancelled", body = InvoiceResponse),
        (status = 400, description = "Cannot cancel - already paid/cancelled"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Not an admin"),
        (status = 404, description = "Invoice not found"),
    )
)]
pub async fn cancel_invoice<A>(
    AdminAuth(_admin): AdminAuth,
    State(state): State<PgAppState<A>>,
    Path(invoice_id): Path<String>,
) -> Result<Json<InvoiceResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let id = InvoiceId::from_string(invoice_id);

    let invoice = InvoiceReader::get(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Can only cancel pending/processing invoices
    match invoice.status {
        InvoiceStatus::Pending | InvoiceStatus::Processing | InvoiceStatus::PartiallyPaid => {}
        _ => return Err(StatusCode::BAD_REQUEST),
    }

    InvoiceWriter::update_status(&*state.data_service, &id, InvoiceStatus::Cancelled)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Deactivate payment options
    if let Err(e) = PaymentOptionWriter::deactivate_for_invoice(&*state.data_service, &id).await {
        tracing::warn!(invoice_id = %id.0, error = %e, "Failed to deactivate payment options");
    }

    let mut cancelled = invoice;
    cancelled.status = InvoiceStatus::Cancelled;

    let options = PaymentOptionReader::get_for_invoice(&*state.data_service, &id)
        .await
        .unwrap_or_default();

    let customer_email = extract_customer_email(&cancelled.metadata);
    let response = InvoiceResponse {
        id: cancelled.id.0,
        currency: cancelled.currency,
        status: cancelled.status.to_string(),
        amount: cancelled.amount,
        amount_received: cancelled.amount_received,
        created_at: cancelled.created_at,
        expires_at: cancelled.expires_at,
        metadata: cancelled.metadata,
        customer_email,
        payment_options: options.into_iter().map(Into::into).collect(),
    };

    // Record metrics
    metrics::record_invoice_cancelled();

    Ok(Json(response))
}
