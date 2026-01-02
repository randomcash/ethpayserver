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

use chrono::Utc;
use clap::Parser;
use data_service::{
    LiveWatchedAddressReader, LiveWatchedAddressWriter, RedisDataService,
};
use evm::error::{EvmError, EvmResult};
use evm::monitor::bridge::{EventBridge, RedisBridge};
use evm::monitor::events::{AddressUnwatched, AddressWatched, StatusReport, WatchedAddressInfo};
use evm::monitor::{
    ChainMonitor, ChainMonitorConfig, CoordinatorConfig, EventHandler, LoggingHandler,
    MonitorCommand, MonitorCoordinator, MonitorEvent, RpcBlockSource, WatchedAddress,
};
use evm::network::get_any_chain_config;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::signal;
use tokio_stream::StreamExt;
use tracing::{debug, error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use types::InvoiceId;

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

    /// Redis channel for events (monitor -> API server)
    #[arg(long, env = "EVMMONITOR_EVENTS_CHANNEL", default_value = "evmmonitor:events")]
    events_channel: String,

    /// Redis channel for commands (API server -> monitor)
    #[arg(long, env = "EVMMONITOR_COMMANDS_CHANNEL", default_value = "evmmonitor:commands")]
    commands_channel: String,

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
    bridge: BridgeConfigFile,
    #[serde(default)]
    chains: Vec<ChainRpcConfig>,
}

#[derive(Debug, Deserialize, Default)]
struct BridgeConfigFile {
    redis_url: Option<String>,
    events_channel: Option<String>,
    commands_channel: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct ChainRpcConfig {
    chain_id: u64,
    rpc_http: String,
    rpc_ws: Option<String>,
}

/// Redis key for publishing health information.
const HEALTH_KEY: &str = "evmmonitor:health";

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

    let events_channel = config.bridge.events_channel.clone()
        .unwrap_or_else(|| args.events_channel.clone());
    let commands_channel = config.bridge.commands_channel.clone()
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
        handle_commands(commands_stream, command_coordinator, command_bridge, command_persistence).await;
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

/// Restore watched addresses from Redis on startup.
async fn restore_watched_addresses(
    coordinator: &Arc<MonitorCoordinator<RpcBlockSource>>,
    persistence: &Arc<RedisDataService>,
    monitored_chain_ids: &[u64],
) {
    match persistence.get_all_watched().await {
        Ok(addresses) => {
            let mut restored_count = 0;
            for (address, invoice_id, chain_id, token_address) in addresses {
                // Only restore addresses for chains we're monitoring
                if !monitored_chain_ids.contains(&chain_id) {
                    debug!(chain_id, address = %address, "skipping address for unmonitored chain");
                    continue;
                }

                if let Some(monitor) = coordinator.get_chain(chain_id).await {
                    // Parse address - it's stored as lowercase hex string
                    let parsed_address = match address.parse() {
                        Ok(addr) => addr,
                        Err(e) => {
                            warn!(address = %address, error = %e, "failed to parse address");
                            continue;
                        }
                    };

                    // Parse invoice_id as UUID
                    let uuid = match uuid::Uuid::parse_str(invoice_id.as_str()) {
                        Ok(u) => u,
                        Err(e) => {
                            warn!(invoice_id = %invoice_id, error = %e, "failed to parse invoice_id as UUID");
                            continue;
                        }
                    };

                    // Parse token contract address if present
                    let token_contract: Option<alloy::primitives::Address> = token_address.as_ref().and_then(|t| t.parse().ok());

                    let watched = WatchedAddress {
                        address: parsed_address,
                        invoice_id: uuid,
                        expected_amount: None,
                        token_contract,
                        created_at: Utc::now(),
                        last_known_balance: alloy::primitives::U256::ZERO,
                    };
                    monitor.watch(watched).await;
                    restored_count += 1;
                }
            }
            info!(count = restored_count, "restored watched addresses from persistence");
        }
        Err(e) => {
            warn!(error = %e, "failed to restore watched addresses from persistence");
        }
    }
}

/// Handle incoming commands from the API server.
async fn handle_commands(
    mut commands: evm::monitor::bridge::CommandStream,
    coordinator: Arc<MonitorCoordinator<RpcBlockSource>>,
    bridge: Arc<dyn EventBridge>,
    persistence: Arc<RedisDataService>,
) {
    while let Some(command) = commands.next().await {
        debug!(?command, "received command");

        match command {
            MonitorCommand::WatchAddress(cmd) => {
                info!(
                    chain_id = cmd.chain_id,
                    address = %cmd.address,
                    invoice_id = %cmd.invoice_id,
                    "watch address command"
                );

                // Persist to Redis first (using chain_id directly for testnet support)
                let invoice_id = InvoiceId::from_string(cmd.invoice_id.to_string());
                let address_str = cmd.address.to_string().to_lowercase();
                let token_address_str = cmd.token_contract.map(|a| a.to_string().to_lowercase());
                if let Err(e) = persistence.watch_address(
                    &address_str,
                    &invoice_id,
                    cmd.chain_id,
                    token_address_str.as_deref(),
                ).await {
                    error!(error = %e, "failed to persist watched address");
                }

                // Find the chain monitor
                if let Some(monitor) = coordinator.get_chain(cmd.chain_id).await {
                    let watched = WatchedAddress {
                        address: cmd.address,
                        invoice_id: cmd.invoice_id.clone(),
                        expected_amount: cmd.expected_amount,
                        token_contract: cmd.token_contract,
                        created_at: Utc::now(),
                        last_known_balance: alloy::primitives::U256::ZERO,
                    };

                    monitor.watch(watched).await;

                    // Publish confirmation event
                    let event = MonitorEvent::AddressWatched(AddressWatched {
                        chain_id: cmd.chain_id,
                        address: cmd.address,
                        invoice_id: cmd.invoice_id,
                        watched_at: Utc::now(),
                    });

                    if let Err(e) = bridge.publish(&event).await {
                        error!(error = %e, "failed to publish AddressWatched event");
                    }
                } else {
                    warn!(
                        chain_id = cmd.chain_id,
                        "chain not monitored by this instance"
                    );
                }
            }

            MonitorCommand::UnwatchAddress(cmd) => {
                info!(
                    chain_id = cmd.chain_id,
                    address = %cmd.address,
                    "unwatch address command"
                );

                // Remove from persistence (using chain_id directly for testnet support)
                let address_str = cmd.address.to_string().to_lowercase();
                let token_address_str = cmd.token_contract.map(|a| a.to_string().to_lowercase());
                if let Err(e) = persistence.unwatch_address(&address_str, cmd.chain_id, token_address_str.as_deref()).await {
                    debug!(error = %e, "failed to remove watched address from persistence");
                }

                if let Some(monitor) = coordinator.get_chain(cmd.chain_id).await {
                    let removed = monitor.unwatch(&cmd.address, cmd.token_contract).await;

                    // Publish confirmation event
                    let event = MonitorEvent::AddressUnwatched(AddressUnwatched {
                        chain_id: cmd.chain_id,
                        address: cmd.address,
                        invoice_id: removed.map(|w| w.invoice_id),
                    });

                    if let Err(e) = bridge.publish(&event).await {
                        error!(error = %e, "failed to publish AddressUnwatched event");
                    }
                } else {
                    warn!(
                        chain_id = cmd.chain_id,
                        "chain not monitored by this instance"
                    );
                }
            }

            MonitorCommand::GetStatus(cmd) => {
                info!(chain_id = ?cmd.chain_id, "get status command");

                let chain_ids = if let Some(chain_id) = cmd.chain_id {
                    vec![chain_id]
                } else {
                    coordinator.chain_ids().await
                };

                for chain_id in chain_ids {
                    if let Some(monitor) = coordinator.get_chain(chain_id).await {
                        let watched = monitor.watched_addresses().await;
                        let current_block = monitor.current_block().await.unwrap_or(0);

                        let event = MonitorEvent::StatusReport(StatusReport {
                            chain_id,
                            watched_count: watched.len(),
                            current_block,
                            addresses: watched
                                .into_iter()
                                .map(|w| WatchedAddressInfo {
                                    address: w.address,
                                    invoice_id: w.invoice_id,
                                    token_contract: w.token_contract,
                                })
                                .collect(),
                            reported_at: Utc::now(),
                        });

                        if let Err(e) = bridge.publish(&event).await {
                            error!(error = %e, "failed to publish StatusReport event");
                        }
                    }
                }
            }
        }
    }

    warn!("commands stream ended");
}

/// Periodically publish health information to Redis.
///
/// Health info is stored as JSON in the HEALTH_KEY with a 60-second TTL.
/// This allows the API server to read health without direct communication.
async fn publish_health_loop(
    coordinator: &Arc<MonitorCoordinator<RpcBlockSource>>,
    redis_url: &str,
) {
    let client = match redis::Client::open(redis_url) {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "failed to connect to redis for health publishing");
            return;
        }
    };

    // Get connection once and reuse it (ConnectionManager handles reconnects)
    let mut conn = match client.get_multiplexed_async_connection().await {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "failed to get redis connection for health publishing");
            return;
        }
    };

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
    info!("health publisher started");

    loop {
        interval.tick().await;

        let health = coordinator.get_all_health().await;

        let health_json = match serde_json::to_string(&health) {
            Ok(j) => j,
            Err(e) => {
                warn!(error = %e, "failed to serialize health");
                continue;
            }
        };

        // SET with 60 second expiry
        if let Err(e) = redis::cmd("SETEX")
            .arg(HEALTH_KEY)
            .arg(60)
            .arg(&health_json)
            .query_async::<()>(&mut conn)
            .await
        {
            warn!(error = %e, "failed to publish health to redis");
            // Try to reconnect on next iteration
            match client.get_multiplexed_async_connection().await {
                Ok(new_conn) => {
                    conn = new_conn;
                    debug!("reconnected to redis for health publishing");
                }
                Err(e) => {
                    warn!(error = %e, "failed to reconnect to redis");
                }
            }
        }
    }
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

    let chain_config = get_any_chain_config(rpc_config.chain_id)
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
