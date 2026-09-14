//! EVM Monitor binary - monitors multiple EVM chains for payments.
//!
//! This binary can be run as a standalone service that monitors blockchain
//! transactions and publishes events to a Redis bridge. Multiple instances
//! can run in parallel, each handling a subset of chains.
//!
//! ## Bidirectional Communication
//!
//! The monitor supports bidirectional communication via Redis:
//!
//! - **Events** (monitor -> API server): PaymentDetected, PaymentConfirmed, etc.
//! - **Commands** (API server -> monitor): WatchAddress, UnwatchAddress, GetStatus
//!
//! This allows the API server to dynamically add/remove watched addresses without
//! restarting the monitor.
//!
//! # Configuration
//!
//! Configure via TOML file or environment variables:
//!
//! ```toml
//! # evmmonitor.toml
//! [bridge]
//! redis_url = "redis://localhost:6379"
//! events_channel = "evmmonitor:events"
//! commands_channel = "evmmonitor:commands"
//!
//! [[chains]]
//! chain_id = 1
//! rpc_http = "https://eth.llamarpc.com"
//! rpc_ws = "wss://eth.llamarpc.com"
//!
//! [[chains]]
//! chain_id = 137
//! rpc_http = "https://polygon-rpc.com"
//! ```
//!
//! Or via environment:
//! ```bash
//! EVMMONITOR_REDIS_URL=redis://localhost:6379
//! EVMMONITOR_CHAINS=1,137
//! EVMMONITOR_CHAIN_1_RPC_HTTP=https://eth.llamarpc.com
//! EVMMONITOR_CHAIN_1_RPC_WS=wss://eth.llamarpc.com
//! EVMMONITOR_CHAIN_137_RPC_HTTP=https://polygon-rpc.com
//! ```

mod chain;
mod commands;
mod config;
mod health;

use std::borrow::Cow;
use std::sync::Arc;

use clap::Parser;
use data_service::RedisDataService;
use evm::error::EvmResult;
use evm::monitor::bridge::{EventBridge, RedisBridge};
use evm::monitor::{
    CoordinatorConfig, EventHandler, LoggingHandler, MonitorCoordinator, MonitorEvent,
};
use secrecy::ExposeSecret;
use tokio::signal;
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use chain::create_chain_monitor;
use commands::{handle_commands, restore_watched_addresses};
use config::{Args, get_chain_configs, load_config};
use health::publish_health_loop;

/// Handler that publishes events to the Redis bridge.
struct BridgeHandler {
    bridge: Arc<dyn EventBridge>,
}

impl BridgeHandler {
    fn new(bridge: Arc<dyn EventBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait::async_trait]
impl EventHandler for BridgeHandler {
    async fn handle(&self, event: &MonitorEvent) -> EvmResult<()> {
        self.bridge.publish(event).await
    }

    fn name(&self) -> &str {
        "BridgeHandler"
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env file if present
    let _ = dotenvy::dotenv();

    // Initialize Sentry (no-op when SENTRY_DSN is unset)
    let (_sentry_guard, sentry_dsn_configured) = init_sentry();

    // Parse CLI args
    let args = Args::parse();

    // Initialize logging (includes Sentry layer when DSN is configured)
    init_logging(&args.log_format, &args.log_level)?;

    info!("starting evmmonitor");

    // Report whether error reporting is actually on. Must come after
    // `init_logging`: `info!`/`error!` before that has no subscriber to write
    // to. This is the component that failed silently for 10.5 hours, so it
    // must not also be silently unreported.
    report_sentry_status(sentry_dsn_configured)?;

    // Load configuration
    let config = load_config(&args)?;

    // Merge CLI args with config file
    let redis_url = args
        .redis_url
        .clone()
        .or(config.bridge.redis_url.clone())
        .ok_or_else(|| anyhow::anyhow!("redis_url is required"))?;
    // One explicit expose for the bridge and health publisher below, both of
    // which need an owned 'static value. The URL stays a secret where it is
    // stored — the CLI args and the config-file struct, which both derive Debug.
    let redis_url = redis_url.expose_secret().to_string();

    let events_channel = config
        .bridge
        .events_channel
        .clone()
        .unwrap_or_else(|| args.events_channel.clone());
    let commands_channel = config
        .bridge
        .commands_channel
        .clone()
        .unwrap_or_else(|| args.commands_channel.clone());

    // Get chain configs - merge CLI chains with config file
    let chain_configs = get_chain_configs(&args.chains, &config.chains)?;

    if chain_configs.is_empty() {
        anyhow::bail!("no chains configured");
    }

    info!(
        chains = ?chain_configs.iter().map(|c| c.chain_id).collect::<Vec<_>>(),
        "monitoring chains"
    );

    // Create Redis bridge with bidirectional channels
    let bridge = Arc::new(RedisBridge::new(&redis_url, &events_channel, &commands_channel).await?);
    info!(
        url = %redis_url,
        events = %events_channel,
        commands = %commands_channel,
        "connected to redis"
    );

    // Health check
    bridge.health_check().await?;
    info!("redis bridge health check passed");

    // Create Redis persistence service
    let persistence = Arc::new(RedisDataService::new(&redis_url).await?);
    persistence.health_check().await?;
    info!("redis persistence connected");

    // Create coordinator
    let coordinator = Arc::new(MonitorCoordinator::new(CoordinatorConfig::new()));

    // Register handlers
    coordinator
        .register_handler(Arc::new(LoggingHandler::new()))
        .await;
    coordinator
        .register_handler(Arc::new(BridgeHandler::new(bridge.clone())))
        .await;

    // Add chain monitors
    let monitored_chain_ids: Vec<u64> = chain_configs.iter().map(|c| c.chain_id).collect();
    for chain_config in &chain_configs {
        match create_chain_monitor(chain_config).await {
            Ok(monitor) => {
                coordinator.add_chain(monitor).await?;
                info!(chain_id = chain_config.chain_id, "chain monitor started");
            }
            Err(e) => {
                error!(
                    chain_id = chain_config.chain_id,
                    error = %e,
                    "failed to create chain monitor"
                );
            }
        }
    }

    // Restore watched addresses from Redis persistence
    restore_watched_addresses(&coordinator, &persistence, &monitored_chain_ids).await;

    // Start coordinator
    coordinator.clone().start().await?;
    info!("monitor coordinator started");

    // Subscribe to commands from API server
    let commands_stream = bridge.subscribe_commands().await?;
    info!("subscribed to commands channel");

    // Spawn command handler task
    let command_coordinator = coordinator.clone();
    let command_bridge = bridge.clone();
    let command_persistence = persistence.clone();
    let command_handle = tokio::spawn(async move {
        handle_commands(
            commands_stream,
            command_coordinator,
            command_bridge,
            command_persistence,
        )
        .await;
    });

    // Spawn health publisher task
    let health_coordinator = coordinator.clone();
    let health_redis_url = redis_url.clone();
    let health_handle = tokio::spawn(async move {
        publish_health_loop(&health_coordinator, &health_redis_url).await;
    });

    // Wait for shutdown signal
    shutdown_signal().await;
    info!("shutdown signal received");

    // Abort background tasks
    command_handle.abort();
    health_handle.abort();

    // Graceful shutdown
    coordinator.stop().await?;
    info!("evmmonitor stopped");

    Ok(())
}

/// Initializes Sentry and reports whether a DSN was actually parsed — an
/// unset or unparseable `SENTRY_DSN` both leave the client a no-op, and
/// `dsn_configured` is what tells the caller which happened.
fn init_sentry() -> (sentry::ClientInitGuard, bool) {
    let dsn = std::env::var("SENTRY_DSN")
        .ok()
        .and_then(|s| s.parse().ok());
    let dsn_configured = dsn.is_some();
    let guard = sentry::init(sentry::ClientOptions {
        dsn,
        release: option_env!("CI_COMMIT_SHORT_SHA").map(Cow::from),
        environment: std::env::var("SENTRY_ENVIRONMENT").ok().map(Cow::from),
        // Never attach default PII (IP, cookies, request bodies). This is a
        // payment processor — see `evm::telemetry::scrub_event`.
        send_default_pii: false,
        // Mandatory secret/PII scrubber: redacts wallet keys, mnemonics, JWTs,
        // API keys, emails and on-chain addresses before events leave the host.
        before_send: Some(Arc::new(evm::telemetry::scrub_event)),
        ..Default::default()
    });
    (guard, dsn_configured)
}

/// Log whether error reporting is on, at INFO, always — never the DSN
/// itself. In the `mainnet` environment, no DSN is a startup error: the
/// monitor is the component that already failed silently once, and must not
/// also run with reporting silently off. Every other environment (including
/// unset) logs and continues.
fn report_sentry_status(dsn_configured: bool) -> anyhow::Result<()> {
    let environment = std::env::var("SENTRY_ENVIRONMENT").unwrap_or_else(|_| "dev".to_string());
    match evm::telemetry::reporting_status(dsn_configured, &environment) {
        evm::telemetry::ReportingStatus::Enabled => {
            info!(environment = %environment, "error reporting enabled");
        }
        evm::telemetry::ReportingStatus::DisabledPermitted => {
            info!(
                environment = %environment,
                "error reporting DISABLED (no DSN configured); permitted outside mainnet"
            );
        }
        evm::telemetry::ReportingStatus::DisabledRefused => {
            anyhow::bail!(
                "error reporting DISABLED (no DSN configured) while environment=mainnet; refusing to start"
            );
        }
    }
    Ok(())
}

fn init_logging(format: &str, level: &str) -> anyhow::Result<()> {
    let filter = EnvFilter::try_new(level)?;

    match format {
        "json" => {
            tracing_subscriber::registry()
                .with(filter)
                .with(sentry_tracing::layer())
                .with(tracing_subscriber::fmt::layer().json())
                .init();
        }
        _ => {
            tracing_subscriber::registry()
                .with(filter)
                .with(sentry_tracing::layer())
                .with(tracing_subscriber::fmt::layer())
                .init();
        }
    }

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
