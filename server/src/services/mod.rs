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
    ArtifactError, DEFAULT_CALL_DEADLINE, DEFAULT_MAX_FAILURES, FilterOutcome, PageElement,
    PageError, PageHost, PageRenderer, PluginArtifacts, PluginBootReport, PluginCallError,
    PluginEngine, PluginHost, PluginHostError, PluginInstance, PluginLoadError, PluginRegistry,
    PluginSchema, PluginStatusSnapshot, PluginStorage, PluginStorageError, PluginWasmError, Viewer,
    host_version, load_installed_plugins, report_boot,
};
pub use watch_retry::{WatchRetryConfig, WatchRetryService};
pub use webhook::{WebhookConfig, WebhookService, WebhookSink};
