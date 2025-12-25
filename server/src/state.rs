//! Application state shared across all handlers.

use std::sync::Arc;

use async_trait::async_trait;
use auth::{AuthRepository, AuthService, StoreRoleRepository};
use data_service::{
    InvoiceReader, InvoiceWriter, PaymentReader, PaymentWriter, StoreWalletReader,
    StoreWalletWriter, StoreWebhookReader, TokenReader, TokenWriter, WatchedAddressReader,
    WatchedAddressWriter,
};
use evm::api::EvmDataService;

use crate::services::EVMMonitor;

/// Trait for data service requirements in the application.
///
/// This trait combines all repository traits needed by the API handlers,
/// plus a health check method for monitoring.
#[async_trait]
pub trait AppDataService:
    InvoiceReader
    + InvoiceWriter
    + PaymentReader
    + PaymentWriter
    + TokenReader
    + TokenWriter
    + WatchedAddressReader
    + WatchedAddressWriter
    + StoreWalletReader
    + StoreWalletWriter
    + StoreWebhookReader
    + StoreRoleRepository
    + EvmDataService
    + Send
    + Sync
{
    /// Check database health.
    async fn health_check(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// Shared application state for all API handlers.
///
/// Generic over the data service type `D`, auth repository type `R`,
/// and EVM monitor type `E`.
pub struct AppState<D: AppDataService, R: AuthRepository, E: EVMMonitor> {
    /// Data service for database operations.
    pub data_service: Arc<D>,

    /// Authentication service.
    pub auth_service: Arc<AuthService<R>>,

    /// EVM monitor for sending commands to evmmonitor.
    /// None if Redis is not configured.
    pub evm_monitor: Option<Arc<E>>,
}

// Manual Clone impl since we only need Arc::clone
impl<D: AppDataService, R: AuthRepository, E: EVMMonitor> Clone for AppState<D, R, E> {
    fn clone(&self) -> Self {
        Self {
            data_service: Arc::clone(&self.data_service),
            auth_service: Arc::clone(&self.auth_service),
            evm_monitor: self.evm_monitor.clone(),
        }
    }
}

impl<D: AppDataService, R: AuthRepository, E: EVMMonitor> AppState<D, R, E> {
    /// Create a new application state.
    pub fn new(
        data_service: Arc<D>,
        auth_service: Arc<AuthService<R>>,
        evm_monitor: Option<Arc<E>>,
    ) -> Self {
        Self {
            data_service,
            auth_service,
            evm_monitor,
        }
    }

    /// Convert to EVM API state.
    pub fn to_evm_state(&self) -> evm::api::EvmState<D, R> {
        evm::api::EvmState::new(
            Arc::clone(&self.data_service),
            Arc::clone(&self.auth_service),
        )
    }
}

// Implement AppDataService for PgDataService
#[async_trait]
impl AppDataService for data_service::PgDataService {
    async fn health_check(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.health_check().await.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }
}

/// Convenient type alias for AppState with PgDataService and RedisEVMMonitor.
///
/// Use this in handlers to avoid specifying the full generic types:
/// ```rust,ignore
/// async fn handler(State(state): State<PgAppState<R>>) -> ...
/// ```
pub type PgAppState<R> = AppState<data_service::PgDataService, R, crate::services::RedisEVMMonitor>;
