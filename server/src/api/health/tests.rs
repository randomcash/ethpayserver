#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::cognitive_complexity
)]

use std::collections::HashMap;

use evm::monitor::{ChainHealth, SourceStatus};

use super::{
    ChainHealthInfo, DeepHealthResponse, DependencyHealth, MonitorHealth, ReadinessResponse,
    RpcHealth,
};

// -- Response type serialization tests --

#[test]
fn readiness_ready_omits_failing() {
    let resp = ReadinessResponse {
        status: "ready".to_string(),
        failing: vec![],
    };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["status"], "ready");
    assert!(
        json.get("failing").is_none(),
        "empty failing should be omitted"
    );
}

#[test]
fn readiness_not_ready_includes_failing() {
    let resp = ReadinessResponse {
        status: "not_ready".to_string(),
        failing: vec!["postgres".to_string(), "rpc:56".to_string()],
    };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["status"], "not_ready");
    let failing = json["failing"].as_array().unwrap();
    assert_eq!(failing.len(), 2);
    assert_eq!(failing[0], "postgres");
    assert_eq!(failing[1], "rpc:56");
}

#[test]
fn deep_health_response_serialization() {
    let mut rpcs = HashMap::new();
    rpcs.insert(
        "1".to_string(),
        RpcHealth {
            status: "ok".to_string(),
            latency_ms: 12,
            last_block: Some(20_000_000),
            error: None,
        },
    );
    rpcs.insert(
        "56".to_string(),
        RpcHealth {
            status: "error".to_string(),
            latency_ms: 1000,
            last_block: None,
            error: Some("disconnected".to_string()),
        },
    );

    let resp = DeepHealthResponse {
        build_sha: "abc1234".to_string(),
        version: "0.1.0".to_string(),
        postgres: DependencyHealth {
            status: "ok".to_string(),
            latency_ms: 3,
            error: None,
        },
        redis: DependencyHealth {
            status: "ok".to_string(),
            latency_ms: 1,
            error: None,
        },
        rpcs,
        monitor: MonitorHealth {
            status: "ok".to_string(),
            data_fresh: true,
        },
    };

    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["build_sha"], "abc1234");
    assert_eq!(json["version"], "0.1.0");
    assert_eq!(json["postgres"]["status"], "ok");
    assert_eq!(json["postgres"]["latency_ms"], 3);
    assert!(json["postgres"].get("error").is_none());
    assert_eq!(json["redis"]["status"], "ok");
    assert_eq!(json["rpcs"]["1"]["status"], "ok");
    assert_eq!(json["rpcs"]["1"]["last_block"], 20_000_000);
    assert_eq!(json["rpcs"]["56"]["status"], "error");
    assert_eq!(json["rpcs"]["56"]["error"], "disconnected");
    assert!(json["rpcs"]["56"].get("last_block").is_none());
    assert_eq!(json["monitor"]["status"], "ok");
    assert_eq!(json["monitor"]["data_fresh"], true);
}

#[test]
fn deep_health_no_monitor_configured() {
    let resp = DeepHealthResponse {
        build_sha: "dev".to_string(),
        version: "0.1.0".to_string(),
        postgres: DependencyHealth {
            status: "ok".to_string(),
            latency_ms: 2,
            error: None,
        },
        redis: DependencyHealth {
            status: "ok".to_string(),
            latency_ms: 0,
            error: Some("not configured".to_string()),
        },
        rpcs: HashMap::new(),
        monitor: MonitorHealth {
            status: "ok".to_string(),
            data_fresh: false,
        },
    };

    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["build_sha"], "dev");
    assert_eq!(json["redis"]["error"], "not configured");
    assert!(json["rpcs"].as_object().unwrap().is_empty());
    assert_eq!(json["monitor"]["data_fresh"], false);
}

// -- ChainHealthInfo conversion tests --

#[test]
fn chain_health_connected_conversion() {
    let health = ChainHealth {
        chain_id: 1,
        chain_name: "Ethereum".to_string(),
        status: SourceStatus::Connected,
        current_block: Some(20_000_000),
        last_processed_block: Some(19_999_999),
        watched_addresses: 42,
        is_healthy: true,
    };
    let info: ChainHealthInfo = health.into();
    assert_eq!(info.status, "connected");
    assert!(info.is_healthy);
    assert_eq!(info.watched_addresses, 42);
}

#[test]
fn chain_health_failed_conversion() {
    let health = ChainHealth {
        chain_id: 56,
        chain_name: "BSC".to_string(),
        status: SourceStatus::Failed("rpc timeout".to_string()),
        current_block: None,
        last_processed_block: None,
        watched_addresses: 0,
        is_healthy: false,
    };
    let info: ChainHealthInfo = health.into();
    assert_eq!(info.status, "failed: rpc timeout");
    assert!(!info.is_healthy);
}

#[test]
fn chain_health_disconnected_conversion() {
    let health = ChainHealth {
        chain_id: 137,
        chain_name: "Polygon".to_string(),
        status: SourceStatus::Disconnected,
        current_block: Some(50_000_000),
        last_processed_block: Some(49_999_000),
        watched_addresses: 10,
        is_healthy: false,
    };
    let info: ChainHealthInfo = health.into();
    assert_eq!(info.status, "disconnected");
    assert!(!info.is_healthy);
}

#[test]
fn readiness_response_roundtrip() {
    let resp = ReadinessResponse {
        status: "not_ready".to_string(),
        failing: vec!["redis".to_string()],
    };
    let json = serde_json::to_string(&resp).unwrap();
    let deserialized: ReadinessResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.status, "not_ready");
    assert_eq!(deserialized.failing, vec!["redis"]);
}

#[test]
fn dependency_health_error_field_omitted_when_none() {
    let dep = DependencyHealth {
        status: "ok".to_string(),
        latency_ms: 5,
        error: None,
    };
    let json = serde_json::to_value(&dep).unwrap();
    assert!(json.get("error").is_none());
}

#[test]
fn rpc_health_last_block_omitted_when_none() {
    let rpc = RpcHealth {
        status: "error".to_string(),
        latency_ms: 1000,
        last_block: None,
        error: Some("timeout".to_string()),
    };
    let json = serde_json::to_value(&rpc).unwrap();
    assert!(json.get("last_block").is_none());
    assert_eq!(json["error"], "timeout");
}
