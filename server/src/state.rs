//! Application state shared across all handlers.

use std::sync::Arc;

use async_trait::async_trait;
use auth::StoreRoleRepository;
use data_service::{
    InvoiceReader, InvoiceWriter, PaymentReader, PaymentWriter, StoreWalletReader,
    StoreWalletWriter, StoreWebhookReader, TokenReader, TokenWriter, WatchedAddressReader,
    WatchedAddressWriter,
};
use evm::api::EvmDataService;
use rates::RateProvider;

use crate::api::ws::WsBroadcast;

/// Read-only data service trait for the application.
///
/// Use this bound for handlers that only read from the database.
#[async_trait]
pub trait AppDataServiceReader:
    InvoiceReader
    + PaymentReader
    + TokenReader
    + WatchedAddressReader
    + StoreWalletReader
    + StoreWebhookReader
    + StoreRoleRepository
    + Send
    + Sync
{
    /// Check database health.
    async fn health_check(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// Full data service trait for the application (read + write).
///
/// Use this bound for handlers that modify the database.
#[async_trait]
pub trait AppDataService:
    AppDataServiceReader
    + InvoiceWriter
    + PaymentWriter
    + TokenWriter
    + WatchedAddressWriter
    + StoreWalletWriter
    + EvmDataService
{
}

/// Shared application state for all API handlers.
///
/// Generic over the data service type `D`, auth service type `A`,
/// and EVM monitor type `E`. `D` and `E` bounds are placed on handlers
/// to allow read-only vs read-write separation.
pub struct AppState<D, A, E> {
    /// Data service for database operations.
    pub data_service: Arc<D>,

    /// Authentication service.
    pub auth_service: Arc<A>,

    /// EVM monitor for sending commands to evmmonitor.
    /// None if Redis is not configured.
    pub evm_monitor: Option<Arc<E>>,

    /// Rate provider for fiat-to-crypto conversions.
    pub rate_provider: Arc<dyn RateProvider>,

    /// WebSocket broadcast channel for real-time status updates.
    pub ws_broadcast: Option<Arc<WsBroadcast>>,

    /// Optional CAPTCHA provider for registration endpoints.
    pub captcha_provider: Option<Arc<dyn auth::captcha::CaptchaProvider>>,
}

// Manual Clone impl since we only need Arc::clone
impl<D, A, E> Clone for AppState<D, A, E> {
    fn clone(&self) -> Self {
        Self {
            data_service: Arc::clone(&self.data_service),
            auth_service: Arc::clone(&self.auth_service),
            evm_monitor: self.evm_monitor.clone(),
            rate_provider: Arc::clone(&self.rate_provider),
            ws_broadcast: self.ws_broadcast.clone(),
            captcha_provider: self.captcha_provider.clone(),
        }
    }
}

impl<D, A, E> AppState<D, A, E> {
    /// Create a new application state.
    pub fn new(
        data_service: Arc<D>,
        auth_service: Arc<A>,
        evm_monitor: Option<Arc<E>>,
        rate_provider: Arc<dyn RateProvider>,
    ) -> Self {
        Self {
            data_service,
            auth_service,
            evm_monitor,
            rate_provider,
            ws_broadcast: None,
            captcha_provider: None,
        }
    }
}

impl<D: EvmDataService, A, E> AppState<D, A, E> {
    /// Convert to EVM API state.
    pub fn to_evm_state(&self) -> evm::api::EvmState<D, A> {
        evm::api::EvmState::new(
            Arc::clone(&self.data_service),
            Arc::clone(&self.auth_service),
        )
    }
}

// Implement AppDataServiceReader for PgDataService
#[async_trait]
impl AppDataServiceReader for data_service::PgDataService {
    async fn health_check(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.health_check()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }
}

// Implement AppDataService for PgDataService (marker trait, extends reader)
impl AppDataService for data_service::PgDataService {}

/// Convenient type alias for AppState with PgDataService and RedisEVMMonitor.
///
/// Use this in handlers to avoid specifying the full generic types:
/// ```rust,ignore
/// async fn handler(State(state): State<PgAppState<A>>) -> ...
/// ```
pub type PgAppState<A> = AppState<data_service::PgDataService, A, crate::services::RedisEVMMonitor>;
