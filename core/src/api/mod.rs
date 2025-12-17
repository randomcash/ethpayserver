//! Unified API for ethpayserver.
//!
//! This module combines all API endpoints from different crates into a single router.

use axum::{routing::get, Router};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use auth::AuthRepository;

use crate::state::AppState;

pub mod health;

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
    ),
    components(schemas(
        health::HealthResponse,
    )),
    tags(
        (name = "health", description = "Health check endpoints"),
        (name = "tokens", description = "Token management (from EVM API)"),
        (name = "networks", description = "Network information (from EVM API)"),
    )
)]
pub struct ApiDoc;

/// Create the unified API router.
///
/// Mounts all sub-routers:
/// - `/health` - Health checks
/// - `/evm` - EVM operations (tokens, networks)
/// - `/auth` - Authentication (TODO)
pub fn router<R>(state: AppState<R>, enable_swagger: bool) -> Router
where
    R: AuthRepository + Send + Sync + 'static,
{
    // Health endpoints with AppState
    let health_routes = Router::new()
        .route("/health", get(health::health_check::<R>))
        .route("/health/live", get(health::liveness))
        .route("/health/ready", get(health::readiness::<R>))
        .with_state(state.clone());

    // EVM API has its own state
    let evm_routes = evm::api::router(state.to_evm_state());

    // Combine all routes
    let mut app = Router::new()
        .merge(health_routes)
        .nest("/evm", evm_routes);

    // Add Swagger UI if enabled
    if enable_swagger {
        // Merge EVM API docs with core docs
        let mut openapi = ApiDoc::openapi();
        let evm_openapi = evm::api::EvmApiDoc::openapi();

        // Merge paths from EVM API
        for (path, item) in evm_openapi.paths.paths {
            openapi.paths.paths.insert(format!("/evm{}", path), item);
        }

        // Merge schemas from EVM API
        if let Some(evm_components) = evm_openapi.components {
            let components = openapi.components.get_or_insert_with(Default::default);
            for (name, schema) in evm_components.schemas {
                components.schemas.insert(name, schema);
            }
        }

        app = app.merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", openapi));
    }

    app
}
