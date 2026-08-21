//! CLI args and configuration-file parsing for evmmonitor.

use clap::Parser;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use std::path::PathBuf;
use tracing::warn;

/// Clap value parser for secret-bearing flags.
fn parse_secret(raw: &str) -> Result<SecretString, std::convert::Infallible> {
    Ok(SecretString::from(raw.to_string()))
}

#[derive(Parser, Debug)]
#[command(name = "evmmonitor")]
#[command(about = "EVM chain payment monitor")]
#[command(version)]
pub(crate) struct Args {
    /// Path to configuration file
    #[arg(short, long, env = "EVMMONITOR_CONFIG")]
    pub config: Option<PathBuf>,

    /// Redis URL for event bridge
    ///
    /// Secret: may carry credentials (`redis://user:pass@host`). Clap derives
    /// Debug on this struct, so `SecretString` is what keeps `--help` output
    /// and any `{:?}` of the args from printing it.
    #[arg(long, env = "EVMMONITOR_REDIS_URL", value_parser = parse_secret)]
    pub redis_url: Option<SecretString>,

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
    /// Secret: may carry credentials (`redis://user:pass@host`).
    pub redis_url: Option<SecretString>,
    pub events_channel: Option<String>,
    pub commands_channel: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct ChainRpcConfig {
    pub chain_id: u64,
    /// Secret: provider URLs carry the API key, so the whole URL is sensitive.
    /// `SecretString` keeps the derived `Debug` above from printing it.
    pub rpc_http: SecretString,
    pub rpc_ws: Option<SecretString>,
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
    get_chain_configs_from(chains_arg, config_chains, |key| std::env::var(key).ok())
}

/// `get_chain_configs` with the environment injected, so the merge rules are
/// testable without mutating process-global state.
fn get_chain_configs_from(
    chains_arg: &Option<String>,
    config_chains: &[ChainRpcConfig],
    env: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Vec<ChainRpcConfig>> {
    let mut configs = config_chains.to_vec();

    // A blank `rpc_http` in the config file is the same dead end as a blank env
    // var, but it cannot be a deploy-time accident — fail loudly instead.
    for chain in &configs {
        if chain.rpc_http.expose_secret().trim().is_empty() {
            anyhow::bail!(
                "chain {} has an empty rpc_http in the config file",
                chain.chain_id
            );
        }
    }

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

            match non_empty(env(&http_key)) {
                Some(rpc_http) => configs.push(ChainRpcConfig {
                    chain_id,
                    rpc_http: SecretString::from(rpc_http),
                    rpc_ws: non_empty(env(&ws_key)).map(SecretString::from),
                }),
                // Set-but-empty is what a deploy that cannot read its .env
                // produces (compose substitutes `${VAR:-}` as ""), so name that
                // case rather than reporting it as "not configured".
                None if env(&http_key).is_some() => warn!(
                    chain_id,
                    env_var = %http_key,
                    "RPC URL for chain is set but empty — skipping chain"
                ),
                None => warn!(
                    chain_id,
                    env_var = %http_key,
                    "no RPC URL configured for chain"
                ),
            }
        }
    }

    Ok(configs)
}

/// Treat a whitespace-only value as absent, and hand back the trimmed rest.
///
/// An empty RPC URL used to travel all the way to `Url::parse`, where it failed
/// as "invalid HTTP URL: relative URL without a base" — a parser error that
/// says nothing about the env var that was actually missing (RCS-196).
fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    fn chain(chain_id: u64, rpc_http: &str) -> ChainRpcConfig {
        ChainRpcConfig {
            chain_id,
            rpc_http: SecretString::from(rpc_http.to_string()),
            rpc_ws: None,
        }
    }

    #[test]
    fn env_chain_is_loaded_with_http_and_ws() {
        let env = env_from(&[
            ("EVMMONITOR_CHAIN_11155111_RPC_HTTP", "https://sepolia/x"),
            ("EVMMONITOR_CHAIN_11155111_RPC_WS", "wss://sepolia/x"),
        ]);

        let configs =
            get_chain_configs_from(&Some("11155111".to_string()), &[], env).expect("configs");

        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].chain_id, 11155111);
        assert_eq!(configs[0].rpc_http.expose_secret(), "https://sepolia/x");
        assert_eq!(
            configs[0].rpc_ws.as_ref().map(|w| w.expose_secret()),
            Some("wss://sepolia/x")
        );
    }

    /// RCS-196: an unreadable .env makes compose substitute every `${VAR:-}` as
    /// "", which used to reach `Url::parse` as an unhelpful parser error.
    #[test]
    fn empty_http_env_var_skips_the_chain() {
        let env = env_from(&[
            ("EVMMONITOR_CHAIN_11155111_RPC_HTTP", ""),
            ("EVMMONITOR_CHAIN_11155111_RPC_WS", ""),
        ]);

        let configs =
            get_chain_configs_from(&Some("11155111".to_string()), &[], env).expect("configs");

        assert!(configs.is_empty());
    }

    #[test]
    fn whitespace_is_trimmed_and_an_empty_ws_url_is_not_configured() {
        let env = env_from(&[
            ("EVMMONITOR_CHAIN_1_RPC_HTTP", "  https://eth/x  "),
            ("EVMMONITOR_CHAIN_1_RPC_WS", "   "),
        ]);

        let configs = get_chain_configs_from(&Some("1".to_string()), &[], env).expect("configs");

        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].rpc_http.expose_secret(), "https://eth/x");
        assert!(configs[0].rpc_ws.is_none());
    }

    #[test]
    fn a_blank_chain_does_not_hide_the_configured_ones() {
        let env = env_from(&[
            ("EVMMONITOR_CHAIN_11155111_RPC_HTTP", "https://sepolia/x"),
            ("EVMMONITOR_CHAIN_80002_RPC_HTTP", ""),
        ]);

        let configs = get_chain_configs_from(&Some("11155111, 80002".to_string()), &[], env)
            .expect("configs");

        let ids: Vec<u64> = configs.iter().map(|c| c.chain_id).collect();
        assert_eq!(ids, vec![11155111]);
    }

    #[test]
    fn config_file_chains_win_over_the_environment() {
        let env = env_from(&[("EVMMONITOR_CHAIN_1_RPC_HTTP", "https://from-env/x")]);
        let file = vec![chain(1, "https://from-file/x")];

        let configs = get_chain_configs_from(&Some("1".to_string()), &file, env).expect("configs");

        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].rpc_http.expose_secret(), "https://from-file/x");
    }

    #[test]
    fn an_empty_rpc_http_in_the_config_file_is_an_error() {
        let file = vec![chain(1, "   ")];

        let err = get_chain_configs_from(&None, &file, env_from(&[])).expect_err("should fail");

        assert!(err.to_string().contains("empty rpc_http"), "{err}");
    }
}
