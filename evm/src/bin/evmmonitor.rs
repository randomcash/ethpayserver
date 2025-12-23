//! EVM Monitor binary - monitors multiple EVM chains for payments.
//!
//! This binary can be run as a standalone service that monitors blockchain
//! transactions and publishes events to a Redis bridge. Multiple instances
//! can run in parallel, each handling a subset of chains.
//!
//! # Configuration
//!
//! Configure via TOML file or environment variables:
//!
//! ```toml
//! # evmmonitor.toml
//! [bridge]
//! redis_url = "redis://localhost:6379"
//! channel = "evmmonitor:events"
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
//! EVMMONITOR_CHAIN_1_RPC_HTTP=https://eth.llamarpc.com
//! EVMMONITOR_CHAIN_1_RPC_WS=wss://eth.llamarpc.com
//! EVMMONITOR_CHAIN_137_RPC_HTTP=https://polygon-rpc.com
//! ```

use clap::Parser;
use evm::error::{EvmError, EvmResult};
use evm::monitor::bridge::{EventBridge, RedisBridge};
use evm::monitor::{
    ChainMonitor, ChainMonitorConfig, CoordinatorConfig, EventHandler, LoggingHandler,
    MonitorCoordinator, MonitorEvent, RpcBlockSource,
};
use evm::network::get_chain_config_by_id;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::signal;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Parser, Debug)]
#[command(name = "evmmonitor")]
#[command(about = "EVM chain payment monitor")]
#[command(version)]
struct Args {
    /// Path to configuration file
    #[arg(short, long, env = "EVMMONITOR_CONFIG")]
    config: Option<PathBuf>,

    /// Redis URL for event bridge
    #[arg(long, env = "EVMMONITOR_REDIS_URL")]
    redis_url: Option<String>,

    /// Redis channel for events
    #[arg(long, env = "EVMMONITOR_REDIS_CHANNEL", default_value = "evmmonitor:events")]
    redis_channel: String,

    /// Chain IDs to monitor (comma-separated)
    #[arg(long, env = "EVMMONITOR_CHAINS")]
    chains: Option<String>,

    /// Log format: json or pretty
    #[arg(long, env = "EVMMONITOR_LOG_FORMAT", default_value = "pretty")]
    log_format: String,

    /// Log level
    #[arg(long, env = "RUST_LOG", default_value = "info")]
    log_level: String,
}

/// Configuration file structure.
#[derive(Debug, Deserialize, Default)]
struct Config {
    #[serde(default)]
    bridge: BridgeConfig,
    #[serde(default)]
    chains: Vec<ChainRpcConfig>,
}

#[derive(Debug, Deserialize, Default)]
struct BridgeConfig {
    redis_url: Option<String>,
    channel: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct ChainRpcConfig {
    chain_id: u64,
    rpc_http: String,
    rpc_ws: Option<String>,
}

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

    // Parse CLI args
    let args = Args::parse();

    // Initialize logging
    init_logging(&args.log_format, &args.log_level)?;

    info!("starting evmmonitor");

    // Load configuration
    let config = load_config(&args)?;

    // Merge CLI args with config file
    let redis_url = args
        .redis_url
        .clone()
        .or(config.bridge.redis_url.clone())
        .ok_or_else(|| anyhow::anyhow!("redis_url is required"))?;

    let redis_channel = args.redis_channel.clone();

    // Get chain configs - merge CLI chains with config file
    let chain_configs = get_chain_configs(&args.chains, &config.chains)?;

    if chain_configs.is_empty() {
        anyhow::bail!("no chains configured");
    }

    info!(
        chains = ?chain_configs.iter().map(|c| c.chain_id).collect::<Vec<_>>(),
        "monitoring chains"
    );

    // Create Redis bridge
    let bridge = Arc::new(RedisBridge::new(&redis_url, &redis_channel).await?);
    info!(url = %redis_url, channel = %redis_channel, "connected to redis");

    // Health check
    bridge.health_check().await?;
    info!("redis health check passed");

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
    for chain_config in chain_configs {
        match create_chain_monitor(&chain_config).await {
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

    // Start coordinator
    coordinator.clone().start().await?;
    info!("monitor coordinator started");

    // Wait for shutdown signal
    shutdown_signal().await;
    info!("shutdown signal received");

    // Graceful shutdown
    coordinator.stop().await?;
    info!("evmmonitor stopped");

    Ok(())
}

fn init_logging(format: &str, level: &str) -> anyhow::Result<()> {
    let filter = EnvFilter::try_new(level)?;

    match format {
        "json" => {
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer().json())
                .init();
        }
        _ => {
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer())
                .init();
        }
    }

    Ok(())
}

fn load_config(args: &Args) -> anyhow::Result<Config> {
    if let Some(path) = &args.config {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    } else {
        // Try default location
        if let Ok(content) = std::fs::read_to_string("evmmonitor.toml") {
            let config: Config = toml::from_str(&content)?;
            return Ok(config);
        }
        Ok(Config::default())
    }
}

fn get_chain_configs(
    chains_arg: &Option<String>,
    config_chains: &[ChainRpcConfig],
) -> anyhow::Result<Vec<ChainRpcConfig>> {
    let mut configs = config_chains.to_vec();

    // Parse chain IDs from CLI
    if let Some(chains_str) = chains_arg {
        let chain_ids: Vec<u64> = chains_str
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();

        // Load RPC URLs from environment for each chain
        for chain_id in chain_ids {
            if configs.iter().any(|c| c.chain_id == chain_id) {
                continue; // Already in config file
            }

            let http_key = format!("EVMMONITOR_CHAIN_{}_RPC_HTTP", chain_id);
            let ws_key = format!("EVMMONITOR_CHAIN_{}_RPC_WS", chain_id);

            if let Ok(rpc_http) = std::env::var(&http_key) {
                configs.push(ChainRpcConfig {
                    chain_id,
                    rpc_http,
                    rpc_ws: std::env::var(&ws_key).ok(),
                });
            } else {
                warn!(
                    chain_id,
                    env_var = %http_key,
                    "no RPC URL configured for chain"
                );
            }
        }
    }

    Ok(configs)
}

async fn create_chain_monitor(rpc_config: &ChainRpcConfig) -> EvmResult<Arc<ChainMonitor<RpcBlockSource>>> {
    use evm::monitor::RpcSourceConfig;

    let chain_config = get_chain_config_by_id(rpc_config.chain_id)
        .ok_or_else(|| EvmError::Monitor(format!("unknown chain id: {}", rpc_config.chain_id)))?;

    let source_config = match &rpc_config.rpc_ws {
        Some(ws_url) => RpcSourceConfig::with_websocket(ws_url, &rpc_config.rpc_http, rpc_config.chain_id),
        None => RpcSourceConfig::http_only(&rpc_config.rpc_http, rpc_config.chain_id),
    };

    let source = RpcBlockSource::new(source_config).await?;
    let monitor_config = ChainMonitorConfig::from_chain(chain_config);
    let monitor = ChainMonitor::new(chain_config, source, monitor_config);

    Ok(Arc::new(monitor))
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
