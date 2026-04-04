//! Payment monitoring system for EVM chains.
//!
//! This module provides a flexible architecture for monitoring blockchain
//! transactions across multiple EVM chains and providers.
//!
//! # Architecture
//!
//! ```text
//! evmmonitor binary (one or more instances)
//!     └── MonitorCoordinator
//!             └── ChainMonitor (per chain)
//!                     └── BlockSource (trait - provider abstraction)
//!                             ├── RpcBlockSource (direct WS/HTTP)
//!                             ├── AlchemyBlockSource (Alchemy enhanced APIs)
//!                             └── InfuraBlockSource (Infura APIs)
//!     └── EventBridge (bidirectional communication)
//!             ├── Events: monitor -> API server (PaymentDetected, etc.)
//!             ├── Commands: API server -> monitor (WatchAddress, etc.)
//!             ├── RedisBridge (production - multiple instances)
//!             └── MemoryBridge (testing/single process)
//!
//! ethpayserver API (subscribes to events via bridge)
//! ```
//!
//! # Example - Running as separate binary
//!
//! ```ignore
//! // evmmonitor binary handles chain monitoring
//! // Configure via TOML or environment variables:
//! //   EVMMONITOR_CHAINS=1,137,42161
//! //   EVMMONITOR_REDIS_URL=redis://localhost:6379
//! //   EVMMONITOR_RPC_1=https://eth.llamarpc.com
//! //   EVMMONITOR_WS_1=wss://eth.llamarpc.com
//!
//! // API server subscribes to events:
//! use evm::monitor::bridge::{BridgeConfig, EventBridge};
//!
//! let bridge = BridgeConfig::redis("redis://localhost:6379").build().await?;
//! let mut events = bridge.subscribe().await?;
//!
//! while let Some(event) = events.next().await {
//!     match event {
//!         MonitorEvent::PaymentDetected(p) => { /* update invoice */ }
//!         MonitorEvent::PaymentConfirmed(p) => { /* mark complete */ }
//!         _ => {}
//!     }
//! }
//!
//! // API server sends commands to monitor:
//! use evm::monitor::events::{MonitorCommand, WatchAddressCommand};
//!
//! let cmd = MonitorCommand::WatchAddress(WatchAddressCommand {
//!     chain_id: 1,
//!     address: payment_address,
//!     invoice_id,
//!     expected_amount: Some(amount),
//!     token_contract: None,
//! });
//! bridge.publish_command(&cmd).await?;
//! ```

pub mod bridge;
mod chain;
mod coordinator;
pub mod events;
mod handlers;
mod source;

pub use source::rpc::{RpcBlockSource, RpcSourceConfig};
pub use source::{BlockNotification, BlockSource, ChainHealth, LogFilter, SourceStatus};
// pub use source::alchemy::AlchemyBlockSource;
// pub use source::infura::InfuraBlockSource;

#[cfg(feature = "redis")]
pub use bridge::RedisBridge;
pub use bridge::{
    BridgeConfig, COMMANDS_CHANNEL, CommandStream, EVENTS_CHANNEL, EventBridge, EventStream,
    MemoryBridge,
};

pub use chain::{ChainMonitor, ChainMonitorConfig, WatchedAddress};
pub use coordinator::{CoordinatorConfig, MonitorCoordinator};
pub use events::{
    AddressUnwatched, AddressWatched, GetStatusCommand, MonitorCommand, MonitorEvent,
    PaymentConfirmed, PaymentDetected, ReorgDetected, StatusReport, UnwatchAddressCommand,
    WatchAddressCommand,
};
pub use handlers::{EventHandler, EventHandlerFn, LoggingHandler};
