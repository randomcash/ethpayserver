//! Application services.

pub mod chain_health_metrics;
pub mod email;
pub mod event_consumer;
pub mod evm_monitor;
pub mod invoice_cleanup;
pub mod plugins;
pub mod watch_retry;
pub mod webhook;

pub use chain_health_metrics::{ChainHealthMetricsConfig, ChainHealthMetricsService};
pub use email::{EmailChangeVerificationData, EmailSender, create_email_sender};
pub use event_consumer::{EventConsumer, EventConsumerError};
pub use evm_monitor::{EVMMonitor, EVMMonitorError, RedisEVMMonitor};
pub use invoice_cleanup::{CleanupConfig, CleanupError, CleanupStats, InvoiceCleanupService};
pub use plugins::{
    ArtifactError, DEFAULT_CALL_DEADLINE, DEFAULT_MAX_FAILURES, DEFAULT_MAX_IN_FLIGHT,
    FilterOutcome, PageElement, PageError, PageHost, PageRenderer, PluginArtifacts,
    PluginBootReport, PluginCallError, PluginEngine, PluginHost, PluginHostError, PluginInstance,
    PluginLoadError, PluginPools, PluginRegistry, PluginSchema, PluginStatusSnapshot,
    PluginStorage, PluginStorageError, PluginWasmError, Viewer, host_version,
    invoice_creation_filters, load_installed_plugins, own_store_payment_reporting,
    payment_observers, report_boot,
};
pub use watch_retry::{WatchRetryConfig, WatchRetryService};
pub use webhook::{WebhookConfig, WebhookService, WebhookSink};
