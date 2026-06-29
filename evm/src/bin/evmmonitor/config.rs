//! CLI args and configuration-file parsing for evmmonitor.

use clap::Parser;
use serde::Deserialize;
use std::path::PathBuf;
use tracing::warn;

#[derive(Parser, Debug)]
#[command(name = "evmmonitor")]
#[command(about = "EVM chain payment monitor")]
#[command(version)]
pub(crate) struct Args {
    /// Path to configuration file
    #[arg(short, long, env = "EVMMONITOR_CONFIG")]
    pub config: Option<PathBuf>,

    /// Redis URL for event bridge
    #[arg(long, env = "EVMMONITOR_REDIS_URL")]
    pub redis_url: Option<String>,

    /// Redis channel for events (monitor -> API server)
    #[arg(
        long,
        env = "EVMMONITOR_EVENTS_CHANNEL",
        default_value = "evmmonitor:events"
    )]
    pub events_channel: String,

    /// Redis channel for commands (API server -> monitor)
    #[arg(
        long,
        env = "EVMMONITOR_COMMANDS_CHANNEL",
        default_value = "evmmonitor:commands"
    )]
    pub commands_channel: String,

    /// Chain IDs to monitor (comma-separated)
    #[arg(long, env = "EVMMONITOR_CHAINS")]
    pub chains: Option<String>,

    /// Log format: json or pretty
    #[arg(long, env = "EVMMONITOR_LOG_FORMAT", default_value = "pretty")]
    pub log_format: String,

    /// Log level
    #[arg(long, env = "RUST_LOG", default_value = "info")]
    pub log_level: String,
}

/// Configuration file structure.
#[derive(Debug, Deserialize, Default)]
pub(crate) struct Config {
    #[serde(default)]
    pub bridge: BridgeConfigFile,
    #[serde(default)]
    pub chains: Vec<ChainRpcConfig>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct BridgeConfigFile {
    pub redis_url: Option<String>,
    pub events_channel: Option<String>,
    pub commands_channel: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct ChainRpcConfig {
    pub chain_id: u64,
    pub rpc_http: String,
    pub rpc_ws: Option<String>,
}

pub(crate) fn load_config(args: &Args) -> anyhow::Result<Config> {
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

pub(crate) fn get_chain_configs(
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
