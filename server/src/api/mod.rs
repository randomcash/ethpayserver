//! Unified API for ethpayserver.
//!
//! This module combines all API endpoints from different crates into a single router.

use axum::{Router, routing::get};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use auth::AuthenticationService;

use crate::state::PgAppState;

pub mod extractors;
pub mod health;
pub mod invoices;
pub mod stores;

pub use extractors::{AdminAuth, AuthenticatedUser};

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
        stores::list_wallets,
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
    ),
    components(schemas(
        health::HealthResponse,
        health::ChainsHealthResponse,
        health::ChainHealthInfo,
        stores::CreateStoreRequest,
        stores::UpdateStoreRequest,
        stores::StoreResponse,
        stores::AddMemberRequest,
        stores::UpdateMemberRequest,
        stores::MemberResponse,
        stores::ConfigureWalletRequest,
        stores::WalletResponse,
        stores::ConfigureWebhookRequest,
        stores::WebhookResponse,
        stores::CreatePaymentMethodRequest,
        stores::UpdatePaymentMethodRequest,
        stores::PaymentMethodResponse,
        invoices::CreateInvoiceRequest,
        invoices::InvoiceResponse,
        invoices::InvoiceListResponse,
        invoices::PaymentResponse,
        invoices::PaymentListResponse,
        invoices::PaymentOptionResponse,
        invoices::InvoiceStatusResponse,
    )),
    tags(
        (name = "health", description = "Health check endpoints"),
        (name = "stores", description = "Store management"),
        (name = "invoices", description = "Invoice management"),
        (name = "payments", description = "Payment management"),
        (name = "tokens", description = "Token management (from EVM API)"),
        (name = "networks", description = "Network information (from EVM API)"),
        (name = "auth", description = "Authentication (from Auth API)"),
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
pub fn router<A>(state: PgAppState<A>, enable_swagger: bool) -> Router
where
    A: AuthenticationService + 'static,
{
    use axum::routing::{delete, post, put};

    // Health endpoints with AppState
    let health_routes = Router::new()
        .route("/health", get(health::health_check::<A>))
        .route("/health/live", get(health::liveness))
        .route("/health/ready", get(health::readiness::<A>))
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
        .route("/{store_id}/webhook", get(stores::get_store_webhook::<A>))
        .route("/{store_id}/webhook", put(stores::configure_store_webhook::<A>))
        .route("/{store_id}/webhook", delete(stores::delete_store_webhook::<A>))
        // Payment methods
        .route("/{store_id}/payment-methods", get(stores::list_payment_methods::<A>))
        .route("/{store_id}/payment-methods", post(stores::create_payment_method::<A>))
        .route("/{store_id}/payment-methods/{method_id}", get(stores::get_payment_method::<A>))
        .route("/{store_id}/payment-methods/{method_id}", put(stores::update_payment_method::<A>))
        .route("/{store_id}/payment-methods/{method_id}", delete(stores::delete_payment_method::<A>))
        .with_state(state.clone());

    // Invoice endpoints
    let invoice_routes = Router::new()
        .route("/", get(invoices::list_invoices::<A>))
        .route("/", post(invoices::create_invoice::<A>))
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
        .with_state(state.clone());

    // Wallet endpoints (cross-store)
    let wallet_routes = Router::new()
        .route("/", get(stores::list_wallets::<A>))
        .with_state(state.clone());

    // Payment endpoints (store-scoped)
    let payment_routes = Router::new()
        .route("/", get(invoices::list_payments::<A>))
        .route("/{payment_id}", get(invoices::get_payment::<A>))
        .with_state(state.clone());

    // Auth API from auth crate
    let auth_state = auth::api::AuthState::new(state.auth_service.clone());
    let auth_routes = auth::api::router(auth_state);

    // EVM API has its own state
    let evm_routes = evm::api::router(state.to_evm_state());

    // Combine all routes
    let mut app = Router::new()
        .merge(health_routes)
        .nest("/stores", store_routes)
        .nest("/wallets", wallet_routes)
        .nest("/invoices", invoice_routes)
        .nest("/payments", payment_routes)
        .nest("/auth", auth_routes)
        .nest("/evm", evm_routes);

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

    app
}
