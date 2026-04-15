//! Invoice management API endpoints.
//!
//! All endpoints require authentication. Users can only access invoices
//! for stores they are members of.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use auth::{SessionService, repository::UserStoreRepository};
use data_service::{PaymentOptionReader, PaymentOptionWriter, StorePaymentMethodReader};
use evm::{Address, U256, XpubDeriver};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use types::{
    InvoiceId, InvoiceQueryParams, InvoiceReader, InvoiceStatus, InvoiceWriter, PaymentMethodId,
    PaymentOptionData, PaymentOptionId, PaymentQueryParams, PaymentReader, StoreId,
    StorePaymentMethodWriter, WatchedAddressWriter,
    traits::{InvoiceData, PaymentData},
};

use super::extractors::{AdminAuth, AuthenticatedUser};
use crate::metrics;
use crate::services::EVMMonitor;
use crate::state::PgAppState;
use rates::{RateError, is_fiat_currency};
use types::currency::DEFAULT_INVOICE_EXPIRATION_SECS;

/// Convert an amount to crypto smallest units using the exchange rate.
///
/// # Arguments
/// * `amount` - Amount in invoice currency (e.g., "100.00" USD or "0.5" BTC)
/// * `rate` - Exchange rate: 1 invoice_currency = rate crypto_units
/// * `decimals` - Number of decimals for the crypto asset (e.g., 18 for ETH)
///
/// # Returns
/// Amount in smallest crypto units as a string (e.g., wei for ETH).
fn convert_to_crypto_smallest_unit(
    amount: &str,
    rate: Decimal,
    decimals: u8,
) -> Result<String, &'static str> {
    // Parse amount
    let parsed_amount: Decimal = amount.parse().map_err(|_| "Invalid amount")?;

    // Validate amount is positive
    if parsed_amount <= Decimal::ZERO {
        return Err("Amount must be positive");
    }

    // Calculate crypto amount: amount * rate
    let crypto_amount = parsed_amount
        .checked_mul(rate)
        .ok_or("Overflow in rate multiplication")?;

    // Convert to smallest units by multiplying by 10^decimals
    let smallest_units = multiply_by_decimals(crypto_amount, decimals)?;

    // Round to integer (floor to avoid overpaying)
    decimal_to_integer_string(smallest_units)
}

/// Convert a human-readable amount to smallest units (no rate conversion).
///
/// # Arguments
/// * `amount` - Amount in human-readable format (e.g., "1.5" ETH)
/// * `decimals` - Number of decimals for the asset (e.g., 18 for ETH)
///
/// # Returns
/// Amount in smallest units as a string (e.g., "1500000000000000000" wei).
fn convert_human_to_smallest_unit(amount: &str, decimals: u8) -> Result<String, &'static str> {
    // Parse amount
    let parsed_amount: Decimal = amount.parse().map_err(|_| "Invalid amount")?;

    // Validate amount is positive
    if parsed_amount <= Decimal::ZERO {
        return Err("Amount must be positive");
    }

    // Convert to smallest units
    let smallest_units = multiply_by_decimals(parsed_amount, decimals)?;

    decimal_to_integer_string(smallest_units)
}

/// Multiply a decimal by 10^decimals.
fn multiply_by_decimals(value: Decimal, decimals: u8) -> Result<Decimal, &'static str> {
    let ten = Decimal::from(10);
    let mut multiplier = Decimal::ONE;
    for _ in 0..decimals {
        multiplier = multiplier
            .checked_mul(ten)
            .ok_or("Overflow computing multiplier")?;
    }
    value
        .checked_mul(multiplier)
        .ok_or("Overflow in smallest units calculation")
}

/// Convert a Decimal to an integer string (floor, then stringify).
fn decimal_to_integer_string(value: Decimal) -> Result<String, &'static str> {
    let floored = value.floor();

    if floored.is_sign_negative() {
        return Err("Negative amount after conversion");
    }

    // Try to_u128 first for efficient conversion
    match floored.to_u128() {
        Some(n) => Ok(n.to_string()),
        None => {
            // Value too large for u128 - use mantissa extraction
            let normalized = floored.normalize();
            if normalized.scale() > 0 {
                return Err("Unexpected decimal places in conversion result");
            }
            let mantissa = normalized.mantissa();
            if mantissa < 0 {
                return Err("Negative amount after conversion");
            }
            Ok(mantissa.to_string())
        }
    }
}

/// Fetch invoice and verify user has access (admin or store member).
/// Returns NOT_FOUND for both missing invoices and permission denied (prevents enumeration).
async fn get_invoice_with_permission<A: SessionService>(
    state: &PgAppState<A>,
    user: &auth::UserInfo,
    invoice_id: &InvoiceId,
) -> Result<InvoiceData, StatusCode> {
    let invoice = InvoiceReader::get(&*state.data_service, invoice_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if user.role != auth::Role::ServerAdmin {
        let is_member = state
            .data_service
            .get_user_store(user.id, invoice.store_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .is_some();

        if !is_member {
            return Err(StatusCode::NOT_FOUND);
        }
    }

    Ok(invoice)
}

/// Request to create a new invoice.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateInvoiceRequest {
    /// Store ID this invoice belongs to.
    pub store_id: Uuid,
    /// Invoice currency (e.g., "USD", "ETH", "BTC").
    /// For asset-denominated invoices (testing), use the asset symbol directly.
    pub currency: String,
    /// Amount in the currency's standard unit (e.g., "100.00" for USD, "0.1" for ETH).
    /// For asset-denominated invoices, this is in the asset's smallest unit (wei, satoshi).
    pub amount: String,
    /// Expiration in seconds from now (default: 900 = 15 minutes).
    pub expiration_seconds: Option<u64>,
    /// Optional metadata.
    pub metadata: Option<serde_json::Value>,
    /// Optional webhook URL.
    pub webhook_url: Option<String>,
    /// Optional redirect URL after payment.
    pub redirect_url: Option<String>,
}

/// Payment option response for an invoice.
#[derive(Debug, Serialize, ToSchema)]
pub struct PaymentOptionResponse {
    /// Payment option ID.
    pub id: String,
    /// Payment method ID (e.g., "ETH-1", "USDC-137").
    pub payment_method_id: String,
    /// Chain ID (EIP-155).
    pub chain_id: u64,
    /// Asset symbol.
    pub asset_symbol: String,
    /// Token contract address (for ERC20, null for native).
    pub token_address: Option<String>,
    /// Asset decimals.
    pub decimals: u8,
    /// Payment address.
    pub payment_address: String,
    /// Amount in the asset's smallest unit.
    pub amount: String,
    /// Exchange rate at time of creation.
    pub rate: Option<String>,
    /// Whether this option is active.
    pub is_active: bool,
}

impl From<PaymentOptionData> for PaymentOptionResponse {
    fn from(po: PaymentOptionData) -> Self {
        Self {
            id: po.id.0.to_string(),
            payment_method_id: po.payment_method_id.0,
            chain_id: po.chain_id,
            asset_symbol: po.asset_symbol,
            token_address: po.token_address,
            decimals: po.decimals,
            payment_address: po.payment_address,
            amount: po.amount,
            rate: po.rate,
            is_active: po.is_active,
        }
    }
}

/// Invoice response (network-agnostic).
#[derive(Debug, Serialize, ToSchema)]
pub struct InvoiceResponse {
    /// Invoice ID.
    pub id: String,
    /// Invoice currency (e.g., "USD", "EUR", "ETH").
    pub currency: String,
    /// Status.
    pub status: String,
    /// Requested amount in the invoice currency.
    pub amount: String,
    /// Amount received so far (in invoice currency terms).
    pub amount_received: String,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Expiration timestamp.
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// Metadata.
    pub metadata: Option<serde_json::Value>,
    /// Payment options for this invoice.
    pub payment_options: Vec<PaymentOptionResponse>,
}

/// Payment response.
#[derive(Debug, Serialize, ToSchema)]
pub struct PaymentResponse {
    /// Payment ID.
    pub id: String,
    /// Chain ID (EIP-155).
    pub chain_id: u64,
    /// Invoice ID this payment belongs to.
    pub invoice_id: String,
    /// Transaction hash.
    pub tx_hash: String,
    /// Amount received.
    pub amount: String,
    /// Asset symbol.
    pub asset_symbol: String,
    /// Token contract address (for ERC20).
    pub token_address: Option<String>,
    /// Block number.
    pub block_number: Option<u64>,
    /// Sender address.
    pub from_address: Option<String>,
    /// When the payment was detected.
    pub detected_at: chrono::DateTime<chrono::Utc>,
    /// When the payment was confirmed (None = awaiting confirmation).
    pub confirmed_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether this payment was invalidated by a chain reorg.
    pub reorged: bool,
}

impl From<PaymentData> for PaymentResponse {
    fn from(p: PaymentData) -> Self {
        Self {
            id: p.id.to_string(),
            chain_id: p.chain_id,
            invoice_id: p.invoice_id.0,
            tx_hash: p.tx_hash,
            amount: p.amount,
            asset_symbol: p.asset_symbol,
            token_address: p.token_address,
            block_number: p.block_number,
            from_address: p.from_address,
            detected_at: p.detected_at,
            confirmed_at: p.confirmed_at,
            reorged: p.reorged,
        }
    }
}

/// Paginated payment list response.
#[derive(Debug, Serialize, ToSchema)]
pub struct PaymentListResponse {
    /// Total number of matching payments.
    pub total: i64,
    /// Payments in this page.
    pub payments: Vec<PaymentResponse>,
}

/// Query parameters for listing payments.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListPaymentsQuery {
    /// Filter by store ID.
    pub store_id: Option<Uuid>,
    /// Filter by status (confirmed, pending).
    pub status: Option<String>,
    /// Maximum number of results.
    pub limit: Option<i64>,
    /// Offset for pagination.
    pub offset: Option<i64>,
}

/// Invoice status response with payment details.
#[derive(Debug, Serialize, ToSchema)]
pub struct InvoiceStatusResponse {
    /// Invoice ID.
    pub id: String,
    /// Current status.
    pub status: String,
    /// Requested amount in invoice currency.
    pub amount: String,
    /// Amount received so far.
    pub amount_received: String,
    /// Invoice currency.
    pub currency: String,
    /// Expiration timestamp.
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// Number of payments received.
    pub payment_count: usize,
    /// Number of confirmed payments.
    pub confirmed_count: usize,
    /// Whether the invoice is fully paid.
    pub is_paid: bool,
    /// Whether the invoice is expired.
    pub is_expired: bool,
    /// Payment options for this invoice.
    pub payment_options: Vec<PaymentOptionResponse>,
    /// Payments received for this invoice.
    pub payments: Vec<PaymentResponse>,
}

/// Query parameters for listing invoices.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListInvoicesQuery {
    /// Filter by store ID.
    pub store_id: Option<Uuid>,
    /// Filter by status.
    pub status: Option<String>,
    /// Filter by currency.
    pub currency: Option<String>,
    /// Maximum number of results.
    pub limit: Option<i64>,
    /// Offset for pagination.
    pub offset: Option<i64>,
}

/// Paginated invoice list response.
#[derive(Debug, Serialize, ToSchema)]
pub struct InvoiceListResponse {
    /// Total number of matching invoices.
    pub total: i64,
    /// Invoices in this page.
    pub invoices: Vec<InvoiceResponse>,
}

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

        responses.push(InvoiceResponse {
            id: invoice.id.0,
            currency: invoice.currency,
            status: invoice.status.to_string(),
            amount: invoice.amount,
            amount_received: invoice.amount_received,
            created_at: invoice.created_at,
            expires_at: invoice.expires_at,
            metadata: invoice.metadata,
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
pub async fn create_invoice<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Json(req): Json<CreateInvoiceRequest>,
) -> Result<(StatusCode, Json<InvoiceResponse>), StatusCode>
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
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !has_permission {
        return Err(StatusCode::FORBIDDEN);
    }

    // Get ALL enabled payment methods for the store
    let payment_methods =
        StorePaymentMethodReader::get_enabled_payment_methods(&*state.data_service, req.store_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if payment_methods.is_empty() {
        tracing::warn!("Store {} has no enabled payment methods", req.store_id);
        return Err(StatusCode::BAD_REQUEST);
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
                        StatusCode::BAD_REQUEST
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
                        return Err(StatusCode::SERVICE_UNAVAILABLE);
                    }

                    // Staleness guard: reject rates older than 5 minutes
                    if exchange_rate.is_stale(chrono::Duration::seconds(300)) {
                        tracing::error!(
                            from = %req.currency,
                            to = %payment_method.asset_symbol,
                            timestamp = %exchange_rate.timestamp,
                            "Exchange rate too stale (>5min), rejecting"
                        );
                        return Err(StatusCode::SERVICE_UNAVAILABLE);
                    }

                    // Warn for rates older than 60 seconds
                    if exchange_rate.is_stale(chrono::Duration::seconds(60)) {
                        tracing::warn!(
                            from = %req.currency,
                            to = %payment_method.asset_symbol,
                            timestamp = %exchange_rate.timestamp,
                            "Exchange rate is >60s old, may be slightly outdated"
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
                        StatusCode::BAD_REQUEST
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
                    return Err(StatusCode::SERVICE_UNAVAILABLE);
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
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }

    let expiration_secs = req
        .expiration_seconds
        .unwrap_or(DEFAULT_INVOICE_EXPIRATION_SECS);
    let expires_at = Utc::now() + chrono::Duration::seconds(expiration_secs as i64);

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
        metadata: req.metadata,
        extra: None,
    };

    InvoiceWriter::upsert(&*state.data_service, &invoice)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // Derive payment address from the payment method's xpub
        let deriver = XpubDeriver::from_xpub(&payment_method.xpub)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let address = deriver
            .derive_address(index as u32)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        // Send WatchAddress command to EVM monitor if configured
        if let Some(ref monitor) = state.evm_monitor {
            // Parse invoice ID as UUID
            let invoice_uuid = Uuid::parse_str(&invoice.id.0).map_err(|e| {
                tracing::error!("Failed to parse invoice ID as UUID: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
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

    let response = InvoiceResponse {
        id: invoice.id.0,
        currency: invoice.currency,
        status: invoice.status.to_string(),
        amount: invoice.amount,
        amount_received: invoice.amount_received,
        created_at: invoice.created_at,
        expires_at: invoice.expires_at,
        metadata: invoice.metadata,
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

    let response = InvoiceResponse {
        id: invoice.id.0,
        currency: invoice.currency,
        status: invoice.status.to_string(),
        amount: invoice.amount,
        amount_received: invoice.amount_received,
        created_at: invoice.created_at,
        expires_at: invoice.expires_at,
        metadata: invoice.metadata,
        payment_options: options.into_iter().map(Into::into).collect(),
    };

    Ok(Json(response))
}

/// Get payments for an invoice.
///
/// User must be a member of the store the invoice belongs to.
#[utoipa::path(
    get,
    path = "/invoices/{invoice_id}/payments",
    tag = "invoices",
    security(("bearer_auth" = [])),
    params(
        ("invoice_id" = String, Path, description = "Invoice ID")
    ),
    responses(
        (status = 200, description = "List of payments", body = Vec<PaymentResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Invoice not found"),
    )
)]
pub async fn get_invoice_payments<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(invoice_id): Path<String>,
) -> Result<Json<Vec<PaymentResponse>>, StatusCode>
where
    A: SessionService + 'static,
{
    let id = InvoiceId::from_string(invoice_id);

    // Verify permission
    let _invoice = get_invoice_with_permission(&state, &user, &id).await?;

    let payments = PaymentReader::get_for_invoice(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(payments.into_iter().map(|p| p.into()).collect()))
}

/// List payments with optional filters.
///
/// Users can only see payments for stores they are members of.
/// store_id is required unless user is a server admin.
#[utoipa::path(
    get,
    path = "/payments",
    tag = "payments",
    security(("bearer_auth" = [])),
    params(ListPaymentsQuery),
    responses(
        (status = 200, description = "List of payments", body = PaymentListResponse),
        (status = 400, description = "store_id required"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Not a member of the store"),
    )
)]
pub async fn list_payments<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Query(query): Query<ListPaymentsQuery>,
) -> Result<Json<PaymentListResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    // For non-admins, store_id is required
    let store_id = match query.store_id {
        Some(id) => id,
        None => {
            if user.role != auth::Role::ServerAdmin {
                return Err(StatusCode::BAD_REQUEST);
            }
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

    let mut params = PaymentQueryParams::new();

    if let Some(status) = query.status {
        match status.as_str() {
            "confirmed" => params = params.with_confirmed(true),
            "pending" => params = params.with_confirmed(false),
            _ => return Err(StatusCode::BAD_REQUEST),
        }
    }

    if let Some(limit) = query.limit {
        params = params.with_limit(limit);
    }

    if let Some(offset) = query.offset {
        params = params.with_offset(offset);
    }

    if store_id != uuid::Uuid::nil() {
        params = params.with_store_id(StoreId(store_id));
    }

    let (total, payments) = PaymentReader::query(&*state.data_service, &params)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(PaymentListResponse {
        total,
        payments: payments.into_iter().map(Into::into).collect(),
    }))
}

/// Get a single payment by ID.
///
/// User must be a member of the store the payment's invoice belongs to.
#[utoipa::path(
    get,
    path = "/payments/{payment_id}",
    tag = "payments",
    security(("bearer_auth" = [])),
    params(
        ("payment_id" = Uuid, Path, description = "Payment ID")
    ),
    responses(
        (status = 200, description = "Payment details", body = PaymentResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Payment not found"),
    )
)]
pub async fn get_payment<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(payment_id): Path<Uuid>,
) -> Result<Json<PaymentResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let payment = PaymentReader::get(&*state.data_service, payment_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Look up the invoice to check store membership
    let invoice = InvoiceReader::get(&*state.data_service, &payment.invoice_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if user.role != auth::Role::ServerAdmin {
        let is_member = state
            .data_service
            .get_user_store(user.id, invoice.store_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .is_some();

        if !is_member {
            return Err(StatusCode::NOT_FOUND);
        }
    }

    Ok(Json(payment.into()))
}

/// Get detailed status of an invoice including payments.
///
/// User must be a member of the store the invoice belongs to.
#[utoipa::path(
    get,
    path = "/invoices/{invoice_id}/status",
    tag = "invoices",
    security(("bearer_auth" = [])),
    params(
        ("invoice_id" = String, Path, description = "Invoice ID")
    ),
    responses(
        (status = 200, description = "Invoice status with payment details", body = InvoiceStatusResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Invoice not found"),
    )
)]
pub async fn get_invoice_status<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(invoice_id): Path<String>,
) -> Result<Json<InvoiceStatusResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let id = InvoiceId::from_string(invoice_id);

    let invoice = get_invoice_with_permission(&state, &user, &id).await?;

    let payments = PaymentReader::get_for_invoice(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let options = PaymentOptionReader::get_for_invoice(&*state.data_service, &id)
        .await
        .unwrap_or_default();

    let now = chrono::Utc::now();
    let confirmed_count = payments.iter().filter(|p| p.confirmed_at.is_some()).count();

    Ok(Json(InvoiceStatusResponse {
        id: invoice.id.0,
        status: invoice.status.to_string(),
        amount: invoice.amount.clone(),
        amount_received: invoice.amount_received,
        currency: invoice.currency,
        expires_at: invoice.expires_at,
        payment_count: payments.len(),
        confirmed_count,
        is_paid: invoice.status == InvoiceStatus::Paid,
        is_expired: invoice.status == InvoiceStatus::Expired || invoice.expires_at < now,
        payment_options: options.into_iter().map(Into::into).collect(),
        payments: payments.into_iter().map(|p| p.into()).collect(),
    }))
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

    let response = InvoiceResponse {
        id: cancelled.id.0,
        currency: cancelled.currency,
        status: cancelled.status.to_string(),
        amount: cancelled.amount,
        amount_received: cancelled.amount_received,
        created_at: cancelled.created_at,
        expires_at: cancelled.expires_at,
        metadata: cancelled.metadata,
        payment_options: options.into_iter().map(Into::into).collect(),
    };

    // Record metrics
    metrics::record_invoice_cancelled();

    Ok(Json(response))
}

/// Generate a payment URI (EIP-681 format for native transfers).
fn _generate_payment_uri(address: &str, amount: &str, _asset_symbol: &str) -> String {
    // For native assets, use simple ethereum: URI
    // For ERC20, would need to add function call data
    format!("ethereum:{}?value={}", address, amount)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_to_crypto_basic() {
        // 100 USD at rate 0.0005 ETH/USD = 0.05 ETH = 50000000000000000 wei
        let rate = Decimal::from_str_exact("0.0005").unwrap();
        let result = convert_to_crypto_smallest_unit("100", rate, 18).unwrap();
        assert_eq!(result, "50000000000000000");
    }

    #[test]
    fn test_convert_to_crypto_small_amount() {
        // 1 USD at rate 0.0005 ETH/USD = 0.0005 ETH = 500000000000000 wei
        let rate = Decimal::from_str_exact("0.0005").unwrap();
        let result = convert_to_crypto_smallest_unit("1", rate, 18).unwrap();
        assert_eq!(result, "500000000000000");
    }

    #[test]
    fn test_convert_to_crypto_large_amount() {
        // 1,000,000 USD at rate 0.0005 ETH/USD = 500 ETH = 500000000000000000000 wei
        let rate = Decimal::from_str_exact("0.0005").unwrap();
        let result = convert_to_crypto_smallest_unit("1000000", rate, 18).unwrap();
        assert_eq!(result, "500000000000000000000");
    }

    #[test]
    fn test_convert_to_crypto_fractional() {
        // 99.99 USD at rate 0.0005 ETH/USD = 0.049995 ETH
        let rate = Decimal::from_str_exact("0.0005").unwrap();
        let result = convert_to_crypto_smallest_unit("99.99", rate, 18).unwrap();
        // 0.049995 ETH = 49995000000000000 wei
        assert_eq!(result, "49995000000000000");
    }

    #[test]
    fn test_convert_to_crypto_usdc_6_decimals() {
        // 100 USD at rate 1.0 USDC/USD = 100 USDC = 100000000 (6 decimals)
        let rate = Decimal::ONE;
        let result = convert_to_crypto_smallest_unit("100", rate, 6).unwrap();
        assert_eq!(result, "100000000");
    }

    #[test]
    fn test_convert_rejects_zero_amount() {
        let rate = Decimal::from_str_exact("0.0005").unwrap();
        let result = convert_to_crypto_smallest_unit("0", rate, 18);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Amount must be positive");
    }

    #[test]
    fn test_convert_rejects_negative_amount() {
        let rate = Decimal::from_str_exact("0.0005").unwrap();
        let result = convert_to_crypto_smallest_unit("-100", rate, 18);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Amount must be positive");
    }

    #[test]
    fn test_convert_rejects_invalid_amount() {
        let rate = Decimal::from_str_exact("0.0005").unwrap();
        let result = convert_to_crypto_smallest_unit("abc", rate, 18);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Invalid amount");
    }

    #[test]
    fn test_convert_floors_result() {
        // Ensure we floor (not round) to avoid overpaying
        // 1.001 USD at rate 0.0005 = 0.0005005 ETH = 500500000000000 wei
        let rate = Decimal::from_str_exact("0.0005").unwrap();
        let result = convert_to_crypto_smallest_unit("1.001", rate, 18).unwrap();
        assert_eq!(result, "500500000000000");
    }

    // Tests for same-asset conversion (no rate needed)
    #[test]
    fn test_convert_human_to_smallest_basic() {
        // 1.5 ETH = 1500000000000000000 wei
        let result = convert_human_to_smallest_unit("1.5", 18).unwrap();
        assert_eq!(result, "1500000000000000000");
    }

    #[test]
    fn test_convert_human_to_smallest_usdc() {
        // 100 USDC = 100000000 (6 decimals)
        let result = convert_human_to_smallest_unit("100", 6).unwrap();
        assert_eq!(result, "100000000");
    }

    #[test]
    fn test_convert_human_to_smallest_fractional() {
        // 0.001 ETH = 1000000000000000 wei
        let result = convert_human_to_smallest_unit("0.001", 18).unwrap();
        assert_eq!(result, "1000000000000000");
    }

    #[test]
    fn test_convert_human_rejects_negative() {
        let result = convert_human_to_smallest_unit("-1", 18);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Amount must be positive");
    }
}
