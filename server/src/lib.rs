//! ETHPayServer core - unified API and service orchestration.
//!
//! This crate provides the main server binary that combines all ethpayserver
//! functionality into a single service.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                     ETHPayServer Core                       │
//! ├─────────────────────────────────────────────────────────────┤
//! │  API Layer (axum)                                           │
//! │  ├── /health     - Health checks                            │
//! │  ├── /evm        - EVM operations (tokens, networks)        │
//! │  ├── /auth       - Authentication (TODO)                    │
//! │  └── /swagger-ui - API documentation                        │
//! ├─────────────────────────────────────────────────────────────┤
//! │  Service Layer                                              │
//! │  ├── AuthService   - User authentication & sessions         │
//! │  └── (PaymentService, InvoiceService - TODO)                │
//! ├─────────────────────────────────────────────────────────────┤
//! │  Data Layer                                                 │
//! │  └── PgDataService - PostgreSQL repositories                │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```bash
//! # Set required environment variables
//! export DATABASE_URL="postgres://user:pass@localhost/ethpayserver"
//!
//! # Run the server
//! cargo run --release
//! ```

pub mod api;
pub mod config;
pub mod services;
pub mod state;

pub use config::Config;
pub use services::{
    CleanupConfig, CleanupError, CleanupStats, InvoiceCleanupService,
    EVMMonitor, EVMMonitorError, EventConsumer, EventConsumerError,
    RedisEVMMonitor, WatchRetryConfig, WatchRetryService,
    WebhookConfig, WebhookService,
};
pub use state::{AppState, AppDataService, AppDataServiceReader};
