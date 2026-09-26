//! Unified API for ethpayserver.
//!
//! This module combines all API endpoints from different crates into a single router.

use std::sync::Arc;

use axum::{
    Router,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use auth::AuthenticationService;

use crate::state::PgAppState;

pub mod admin;
pub mod api_key_deprecation;
pub mod api_key_hash;
pub mod api_key_rate_limit;
pub mod checkout;
pub mod dashboard;
pub mod extractors;
pub mod health;
pub mod http_metrics;
pub mod idempotency;
pub mod invoices;
pub mod payouts;
pub mod plugins;
pub mod rate_limit;
pub mod rates;
pub mod refunds;
pub mod stores;
pub mod users;
pub mod webhook_deliveries;
pub mod ws;

pub use extractors::{
    AdminAuth, AuthenticatedCaller, AuthenticatedUser, FreshlyAuthenticatedUser, StoreScopedUser,
};

/// A status, optionally with a reason the caller can read.
///
/// Handlers mostly return bare `StatusCode`, and that stays true:
/// `From<StatusCode>` gives an empty reason, so `?` on existing permission and
/// lookup helpers is unchanged and those responses keep exactly the shape they
/// had. What this adds is somewhere for a refusal's own words to travel, for
/// the cases where the status alone does not say enough - originally so a
/// repository conflict could say what it refused (see `stores::repository_error`),
/// now shared with the invoice/payment filter builders for the same reason:
/// a bare 400 and a bare "store_id required" 400 are indistinguishable on the
/// wire, which is exactly the ambiguity that burned the client once.
#[derive(Debug)]
pub struct ApiErr(StatusCode, String);

impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        // An empty reason stays a bare status, which is what every handler here
        // returned before and what `From<StatusCode>` produces.
        //
        // Note what this does NOT fix: both branches send an empty body, so the
        // client still Displays a reasonless error as "HTTP error 404: ",
        // trailing colon and all. The difference is only that the bare branch
        // sends no `content-type` for a body that does not exist. Filling the
        // reason is what removes the colon, and that is the caller's job.
        if self.1.is_empty() {
            self.0.into_response()
        } else {
            (self.0, self.1).into_response()
        }
    }
}

impl From<StatusCode> for ApiErr {
    fn from(status: StatusCode) -> Self {
        Self(status, String::new())
    }
}

impl From<(StatusCode, String)> for ApiErr {
    fn from((status, reason): (StatusCode, String)) -> Self {
        Self(status, reason)
    }
}

/// OpenAPI documentation for the entire API.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "ETHPayServer API",
        version = "0.1.0",
        description = "Self-hosted Ethereum payment processor API",
        license(name = "MIT"),
    ),
    paths(
        // Health
        health::health_check,
        health::liveness,
        health::readiness,
        health::deep_health,
        health::chains_health,
        health::prometheus_metrics,
        // Stores
        stores::list_stores,
        stores::create_store,
        stores::get_store,
        stores::update_store,
        stores::delete_store,
        stores::list_store_members,
        stores::add_store_member,
        stores::update_store_member,
        stores::remove_store_member,
        stores::get_store_wallet,
        stores::configure_store_wallet,
        stores::delete_store_wallet,
        stores::rotate_store_wallet,
        stores::list_wallets,
        stores::create_wallet,
        stores::get_wallet_by_id,
        stores::update_wallet,
        stores::delete_wallet,
        stores::export_wallet_xpub,
        stores::list_wallet_addresses,
        stores::get_store_webhook,
        stores::configure_store_webhook,
        stores::delete_store_webhook,
        // Payment Methods
        stores::list_payment_methods,
        stores::create_payment_method,
        stores::get_payment_method,
        stores::update_payment_method,
        stores::delete_payment_method,
        // Invoices
        invoices::list_invoices,
        invoices::create_invoice,
        invoices::get_invoice,
        invoices::get_invoice_payments,
        invoices::get_invoice_status,
        invoices::cancel_invoice,
        // Payments
        invoices::list_payments,
        invoices::get_payment,
        // Dashboard
        dashboard::get_stats,
        dashboard::get_analytics,
        // Rates
        rates::get_rate,
        // Users
        users::list_api_keys,
        users::create_api_key,
        users::revoke_api_key,
        users::update_api_key,
        users::update_api_key_permissions,
        users::rotate_api_key,
        users::list_wallet_credentials,
        users::create_wallet_reauth_challenge,
        users::set_primary_wallet_credential,
        // Admin
        admin::list_users,
        admin::update_user_role,
        admin::lock_user,
        admin::unlock_user,
        admin::get_settings,
        admin::update_settings,
        admin::get_safe_mode,
        plugins::list_plugin_pages,
        admin::plugins::list_plugins,
        admin::plugins::install_plugin,
        admin::plugins::enable_plugin,
        admin::plugins::disable_plugin,
        admin::plugins::uninstall_plugin,
        admin::plugins::plugin_events,
        admin::plugins::cancel_plugin_subscription,
    ),
    components(schemas(
        health::HealthResponse,
        health::ReadinessResponse,
        health::DeepHealthResponse,
        health::DependencyHealth,
        health::RpcHealth,
        health::MonitorHealth,
        health::ChainsHealthResponse,
        health::ChainHealthInfo,
        stores::CreateStoreRequest,
        stores::UpdateStoreRequest,
        stores::StoreResponse,
        stores::AddMemberRequest,
        stores::UpdateMemberRequest,
        stores::MemberResponse,
        stores::CreateWalletRequest,
        stores::UpdateWalletRequest,
        stores::SetStoreWalletRequest,
        stores::StoreWalletResponse,
        stores::MethodWalletResponse,
        stores::WalletResponse,
        stores::CreateWalletResponse,
        stores::WalletXpubResponse,
        stores::DerivedAddressEntry,
        stores::WalletAddressesResponse,
        stores::ConfigureWebhookRequest,
        stores::WebhookResponse,
        stores::CreatePaymentMethodRequest,
        stores::UpdatePaymentMethodRequest,
        stores::PaymentMethodResponse,
        stores::RotateWalletRequest,
        stores::RotateWalletResponse,
        stores::RotationEntry,
        invoices::CreateInvoiceRequest,
        invoices::InvoiceResponse,
        invoices::InvoiceListResponse,
        invoices::PaymentResponse,
        invoices::PaymentListResponse,
        invoices::PaymentOptionResponse,
        invoices::InvoiceStatusResponse,
        dashboard::DashboardStats,
        dashboard::DashboardAnalytics,
        dashboard::AssetVolume,
        dashboard::DailyVolume,
        rates::RateResponse,
        users::ApiKeyListResponse,
        users::ApiKeyInfoResponse,
        users::CreateApiKeyPayload,
        users::CreateApiKeyResponsePayload,
        users::UpdateApiKeyPayload,
        users::UpdateApiKeyPermissionsPayload,
        users::RotateApiKeyResponsePayload,
        users::WalletCredentialResponse,
        users::WalletReauthChallengeResponse,
        users::PromoteWalletCredentialRequest,
        admin::UserListResponse,
        admin::AdminUserInfo,
        admin::UpdateRoleRequest,
        admin::ServerSettingsResponse,
        admin::UpdateServerSettingsRequest,
        admin::SafeModeResponse,
        plugins::PluginPagesResponse,
        plugins::PluginPagesInfo,
        plugins::PluginPageInfo,
        admin::plugins::AdminPluginInfo,
        admin::plugins::AdminPluginListResponse,
        admin::plugins::InstallPluginRequest,
        admin::plugins::DisablePluginRequest,
        admin::plugins::PluginMutationResponse,
        admin::plugins::PluginEventInfo,
        admin::plugins::PluginEventListResponse,
        admin::plugins::CancelSubscriptionResponse,
    )),
    tags(
        (name = "health", description = "Health check endpoints"),
        (name = "stores", description = "Store management"),
        (name = "invoices", description = "Invoice management"),
        (name = "payments", description = "Payment management"),
        (name = "tokens", description = "Token management (from EVM API)"),
        (name = "networks", description = "Network information (from EVM API)"),
        (name = "auth", description = "Authentication (from Auth API)"),
        (name = "dashboard", description = "Dashboard statistics and analytics"),
        (name = "rates", description = "Exchange rates"),
        (name = "users", description = "User management (API keys)"),
        (name = "admin", description = "Server administration"),
    )
)]
pub struct ApiDoc;

/// Create the unified API router.
///
/// Mounts all sub-routers:
/// - `/health` - Health checks
/// - `/evm` - EVM operations (tokens, networks)
/// - `/auth` - Authentication
/// - `/stores` - Store management
#[allow(clippy::too_many_lines)] // route registration table — splitting hides the full API surface
pub fn router<A>(
    state: PgAppState<A>,
    enable_swagger: bool,
    rate_limiters: Option<Arc<rate_limit::RateLimitState>>,
    idempotency: Option<Arc<idempotency::IdempotencyState>>,
    api_key_rate_limiter: Option<Arc<api_key_rate_limit::ApiKeyRateLimitState>>,
) -> Router
where
    A: AuthenticationService + 'static,
{
    use axum::routing::{delete, patch, post, put};

    // Health endpoints with AppState
    let health_routes = Router::new()
        .route("/health", get(health::health_check::<A>))
        .route("/health/live", get(health::liveness))
        .route("/health/ready", get(health::readiness::<A>))
        .route("/health/deep", get(health::deep_health::<A>))
        .route("/health/chains", get(health::chains_health::<A>))
        .route("/metrics", get(health::prometheus_metrics::<A>))
        .with_state(state.clone());

    // Store endpoints
    let store_routes = Router::new()
        .route("/", get(stores::list_stores::<A>))
        .route("/", post(stores::create_store::<A>))
        .route("/{store_id}", get(stores::get_store::<A>))
        .route("/{store_id}", put(stores::update_store::<A>))
        .route("/{store_id}", delete(stores::delete_store::<A>))
        .route("/{store_id}/members", get(stores::list_store_members::<A>))
        .route("/{store_id}/members", post(stores::add_store_member::<A>))
        .route(
            "/{store_id}/members/{user_id}",
            put(stores::update_store_member::<A>),
        )
        .route(
            "/{store_id}/members/{user_id}",
            delete(stores::remove_store_member::<A>),
        )
        .route("/{store_id}/wallet", get(stores::get_store_wallet::<A>))
        .route("/{store_id}/wallet", put(stores::configure_store_wallet::<A>))
        .route("/{store_id}/wallet", delete(stores::delete_store_wallet::<A>))
        .route("/{store_id}/wallet/rotate", post(stores::rotate_store_wallet::<A>))
        .route("/{store_id}/settings", get(stores::get_store_settings::<A>))
        .route("/{store_id}/settings", patch(stores::update_store_settings::<A>))
        .route("/{store_id}/webhook", get(stores::get_store_webhook::<A>))
        .route("/{store_id}/webhook", put(stores::configure_store_webhook::<A>))
        .route("/{store_id}/webhook", delete(stores::delete_store_webhook::<A>))
        // Payment methods
        .route("/{store_id}/payment-methods", get(stores::list_payment_methods::<A>))
        .route("/{store_id}/payment-methods", post(stores::create_payment_method::<A>))
        .route("/{store_id}/payment-methods/{method_id}", get(stores::get_payment_method::<A>))
        .route("/{store_id}/payment-methods/{method_id}", put(stores::update_payment_method::<A>))
        .route("/{store_id}/payment-methods/{method_id}", delete(stores::delete_payment_method::<A>))
        // Token Policy
        .route("/{store_id}/token-policy", get(stores::get_token_policy::<A>))
        .route("/{store_id}/token-policy", put(stores::set_token_policy::<A>))
        .route("/{store_id}/token-policy", delete(stores::delete_token_policy::<A>))
        // Payouts
        .route("/{store_id}/payouts", get(payouts::list_payouts::<A>))
        .route("/{store_id}/payouts", post(payouts::create_payout::<A>))
        .route("/{store_id}/payouts/{payout_id}", get(payouts::get_payout::<A>))
        // The merchant makes the payout from their own wallet, then records it
        // here; this server has no spending key and never broadcasts.
        .route(
            "/{store_id}/payouts/{payout_id}/settle",
            post(payouts::settle_payout::<A>),
        )
        // Releases the invoice claim a payout holds, so a payout recorded by
        // mistake does not lock that money out of every later payout.
        .route(
            "/{store_id}/payouts/{payout_id}/abandon",
            post(payouts::abandon_payout::<A>),
        )
        // Webhook deliveries
        .route(
            "/{store_id}/webhook-deliveries",
            get(webhook_deliveries::list_deliveries_for_store::<A>),
        )
        .route(
            "/{store_id}/webhook-deliveries/{delivery_id}/replay",
            post(webhook_deliveries::replay_delivery::<A>),
        )
        .with_state(state.clone());

    // Invoice endpoints (with idempotency middleware on POST)
    let mut invoice_routes = Router::new()
        .route("/", get(invoices::list_invoices::<A>))
        .route("/", post(invoices::create_invoice::<A>))
        .route("/export.csv", get(invoices::export_invoices_csv::<A>))
        .route(
            "/by-tx/{chain_id}/{tx_hash}",
            get(invoices::lookup_by_tx_hash::<A>),
        )
        .route("/{invoice_id}", get(invoices::get_invoice::<A>))
        .route(
            "/{invoice_id}/payments",
            get(invoices::get_invoice_payments::<A>),
        )
        .route(
            "/{invoice_id}/status",
            get(invoices::get_invoice_status::<A>),
        )
        .route("/{invoice_id}/cancel", post(invoices::cancel_invoice::<A>))
        .route("/{invoice_id}/refund", post(refunds::create_refund::<A>))
        .route("/{invoice_id}/refunds", get(refunds::list_refunds::<A>))
        .route(
            "/{invoice_id}/webhook-deliveries",
            get(webhook_deliveries::list_deliveries_for_invoice::<A>),
        )
        .with_state(state.clone());

    if let Some(idem) = idempotency {
        invoice_routes = invoice_routes.layer(axum::middleware::from_fn_with_state(
            idem,
            idempotency::middleware,
        ));
    }

    // Wallet endpoints (cross-store)
    let wallet_routes = Router::new()
        .route("/", get(stores::list_wallets::<A>))
        .route("/", post(stores::create_wallet::<A>))
        .route("/{wallet_id}", get(stores::get_wallet_by_id::<A>))
        .route("/{wallet_id}", patch(stores::update_wallet::<A>))
        .route("/{wallet_id}", delete(stores::delete_wallet::<A>))
        .route("/{wallet_id}/xpub", get(stores::export_wallet_xpub::<A>))
        .route(
            "/{wallet_id}/addresses",
            get(stores::list_wallet_addresses::<A>),
        )
        .with_state(state.clone());

    // Payment endpoints (store-scoped)
    let payment_routes = Router::new()
        .route("/", get(invoices::list_payments::<A>))
        .route("/export.csv", get(invoices::export_payments_csv::<A>))
        .route("/{payment_id}", get(invoices::get_payment::<A>))
        .with_state(state.clone());

    // WebSocket endpoint for real-time updates
    let ws_route = Router::new()
        .route("/ws", get(ws::ws_handler::<A>))
        .with_state(state.clone());

    // Rates endpoint
    let rates_routes = Router::new()
        .route("/", get(rates::get_rate::<A>))
        .with_state(state.clone());

    // Plugin static pages. Same route for merchant and admin views - the
    // handler resolves the viewer from the authenticated identity.
    let plugin_routes = Router::new()
        // What the client puts in its navigation. Declared in each plugin's
        // manifest, so building a menu runs no plugin code.
        .route("/", get(plugins::list_plugin_pages::<A>))
        .route("/{id}/pages/{*path}", get(plugins::get_page::<A>))
        .with_state(state.clone());

    // Dashboard endpoint
    let dashboard_routes = Router::new()
        .route("/stats", get(dashboard::get_stats::<A>))
        .route("/analytics", get(dashboard::get_analytics::<A>))
        .with_state(state.clone());

    // User endpoints (API keys)
    let user_routes = Router::new()
        // Refuses while the account's stores hold payments, payouts or refunds:
        // `users` cascades through `stores` into `invoices` and `payments`, so
        // deleting a merchant who traded would erase their financial history.
        .route("/me", delete(users::delete_account::<A>))
        // Email change (sensitive - see server/src/api/users.rs).
        // Set/change and remove require a fresh passkey or wallet login
        // (`FreshlyAuthenticatedUser`); confirm is unauthenticated by design
        // and gated on the verification token alone.
        .route("/me/email", post(users::request_email_change::<A>))
        .route("/me/email", delete(users::remove_email::<A>))
        .route(
            "/me/email/confirm",
            post(users::confirm_email_change::<A>),
        )
        .route("/api-keys", get(users::list_api_keys::<A>))
        .route("/api-keys", post(users::create_api_key::<A>))
        .route("/api-keys/{id}", delete(users::revoke_api_key::<A>))
        .route(
            "/api-keys/{id}",
            axum::routing::patch(users::update_api_key::<A>),
        )
        .route(
            "/api-keys/{id}/permissions",
            axum::routing::patch(users::update_api_key_permissions::<A>),
        )
        .route(
            "/api-keys/{id}/rotate",
            axum::routing::post(users::rotate_api_key::<A>),
        )
        .route("/wallets", get(users::list_wallet_credentials::<A>))
        .route(
            "/wallets/{id}/reauth-challenge",
            axum::routing::post(users::create_wallet_reauth_challenge::<A>),
        )
        .route(
            "/wallets/{id}/primary",
            axum::routing::patch(users::set_primary_wallet_credential::<A>),
        )
        .with_state(state.clone());
    // Admin endpoints (ServerAdmin only)
    let admin_routes = Router::new()
        .route("/users", get(admin::list_users::<A>))
        .route(
            "/users/{id}/role",
            axum::routing::patch(admin::update_user_role::<A>),
        )
        .route(
            "/users/{id}/lock",
            axum::routing::post(admin::lock_user::<A>),
        )
        .route(
            "/users/{id}/unlock",
            axum::routing::post(admin::unlock_user::<A>),
        )
        .route("/settings", get(admin::get_settings::<A>))
        .route("/settings", axum::routing::put(admin::update_settings::<A>))
        .route("/safe-mode", get(admin::get_safe_mode::<A>))
        .route(
            "/plugins",
            get(admin::plugins::list_plugins::<A>).post(admin::plugins::install_plugin::<A>),
        )
        .route(
            "/plugins/{id}",
            delete(admin::plugins::uninstall_plugin::<A>),
        )
        .route(
            "/plugins/{id}/enable",
            post(admin::plugins::enable_plugin::<A>),
        )
        .route(
            "/plugins/{id}/disable",
            post(admin::plugins::disable_plugin::<A>),
        )
        .route(
            "/plugins/{id}/events",
            get(admin::plugins::plugin_events::<A>),
        )
        .route(
            "/plugins/{id}/accounts/{account_id}/cancel-subscription",
            post(admin::plugins::cancel_plugin_subscription::<A>),
        )
        .with_state(state.clone());

    // Auth API from auth crate (with optional CAPTCHA provider)
    let auth_state = match state.captcha_provider.clone() {
        Some(captcha) => auth::api::AuthState::with_captcha(state.auth_service.clone(), captcha),
        None => auth::api::AuthState::new(state.auth_service.clone()),
    };
    let auth_routes = auth::api::router(auth_state);

    // EVM API has its own state
    let evm_routes = evm::api::router(state.to_evm_state());

    // Public checkout endpoints (no auth required)
    let checkout_routes = Router::new()
        .route("/{invoice_id}", get(checkout::get_checkout::<A>))
        .route("/ws", get(checkout::checkout_ws_handler::<A>))
        .with_state(state.clone());

    // Combine all routes
    let mut app = Router::new()
        .merge(health_routes)
        .nest("/checkout", checkout_routes)
        .nest("/stores", store_routes)
        .nest("/wallets", wallet_routes)
        .nest("/invoices", invoice_routes)
        .nest("/payments", payment_routes)
        .merge(ws_route)
        .nest("/rates", rates_routes)
        .nest("/plugins", plugin_routes)
        .nest("/dashboard", dashboard_routes)
        .nest("/users", user_routes)
        .nest("/admin", admin_routes)
        .nest("/auth", auth_routes)
        .nest("/evm", evm_routes);

    // Apply per-API-key rate limiting (runs after IP-tier, both must pass)
    if let Some(api_key_limiter) = api_key_rate_limiter {
        app = app.layer(axum::middleware::from_fn_with_state(
            api_key_limiter,
            api_key_rate_limit::middleware,
        ));
    }

    // Apply IP-tier rate limiting middleware if configured
    if let Some(limiters) = rate_limiters {
        app = app.layer(axum::middleware::from_fn_with_state(
            limiters,
            rate_limit::middleware,
        ));
    }

    // Add Swagger UI if enabled
    if enable_swagger {
        // Merge all API docs
        let mut openapi = ApiDoc::openapi();
        let evm_openapi = evm::api::EvmApiDoc::openapi();
        let auth_openapi = auth::AuthApiDoc::openapi();

        // Merge paths from EVM API
        for (path, item) in evm_openapi.paths.paths {
            openapi.paths.paths.insert(format!("/evm{}", path), item);
        }

        // Merge paths from Auth API
        for (path, item) in auth_openapi.paths.paths {
            openapi.paths.paths.insert(format!("/auth{}", path), item);
        }

        // Merge schemas from EVM API
        if let Some(evm_components) = evm_openapi.components {
            let components = openapi.components.get_or_insert_with(Default::default);
            for (name, schema) in evm_components.schemas {
                components.schemas.insert(name, schema);
            }
        }

        // Merge schemas from Auth API
        if let Some(auth_components) = auth_openapi.components {
            let components = openapi.components.get_or_insert_with(Default::default);
            for (name, schema) in auth_components.schemas {
                components.schemas.insert(name, schema);
            }
        }

        app = app.merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", openapi));
    }

    // API-key deprecation response header — runs on every response so any
    // handler that was reached via a deprecated-but-in-grace key gets the
    // `X-API-Key-Deprecated` header stamped automatically.
    app = app.layer(axum::middleware::from_fn(api_key_deprecation::middleware));

    // HTTP request metrics (innermost layer = runs closest to handlers)
    app = app.layer(axum::middleware::from_fn(http_metrics::middleware));

    app
}
