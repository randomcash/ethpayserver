//! Application state shared across all handlers.

use std::sync::Arc;

use auth::{AuthRepository, AuthService};
use data_service::PgDataService;

use crate::services::EVMMonitor;

/// Shared application state for all API handlers.
///
/// Generic over the auth repository type `R`.
pub struct AppState<R: AuthRepository> {
    /// PostgreSQL data service.
    pub data_service: Arc<PgDataService>,

    /// Authentication service.
    pub auth_service: Arc<AuthService<R>>,

    /// EVM monitor for sending commands to evmmonitor.
    /// None if Redis is not configured.
    pub evm_monitor: Option<Arc<dyn EVMMonitor>>,
}

// Manual Clone impl since we only need Arc::clone, not R: Clone
impl<R: AuthRepository> Clone for AppState<R> {
    fn clone(&self) -> Self {
        Self {
            data_service: Arc::clone(&self.data_service),
            auth_service: Arc::clone(&self.auth_service),
            evm_monitor: self.evm_monitor.clone(),
        }
    }
}

impl<R: AuthRepository> AppState<R> {
    /// Create a new application state.
    pub fn new(
        data_service: Arc<PgDataService>,
        auth_service: Arc<AuthService<R>>,
        evm_monitor: Option<Arc<dyn EVMMonitor>>,
    ) -> Self {
        Self {
            data_service,
            auth_service,
            evm_monitor,
        }
    }

    /// Convert to EVM API state.
    pub fn to_evm_state(&self) -> evm::api::EvmState<R> {
        evm::api::EvmState::new(
            Arc::clone(&self.data_service),
            Arc::clone(&self.auth_service),
        )
    }
}
