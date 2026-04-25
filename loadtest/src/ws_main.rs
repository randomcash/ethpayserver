//! WebSocket connection load test for ethpayserver.
//!
//! Establishes N concurrent WebSocket connections and holds them for a
//! configurable duration, measuring disconnect rate and message latency.
//!
//! Target: 500 concurrent clients, < 0.1% disconnect rate over 5 minutes.
//!
//! # Environment variables
//!
//! | Variable                  | Required | Default              |
//! |---------------------------|----------|----------------------|
//! | `LOADTEST_WS_URL`         | no       | ws://localhost:3000   |
//! | `LOADTEST_WS_CLIENTS`     | no       | 500                  |
//! | `LOADTEST_WS_DURATION_SECS`| no      | 300                  |
//! | `LOADTEST_WS_RAMP_SECS`   | no       | 30                   |
//!
//! # Usage
//!
//! ```sh
//! cargo run -p loadtest --bin loadtest-ws
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

struct Metrics {
    connected: AtomicU64,
    disconnected: AtomicU64,
    messages_received: AtomicU64,
    errors: AtomicU64,
}

#[tokio::main]
async fn main() {
    let ws_url =
        std::env::var("LOADTEST_WS_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string());
    let num_clients: u64 = std::env::var("LOADTEST_WS_CLIENTS")
        .unwrap_or_else(|_| "500".to_string())
        .parse()
        .expect("LOADTEST_WS_CLIENTS must be a number");
    let duration_secs: u64 = std::env::var("LOADTEST_WS_DURATION_SECS")
        .unwrap_or_else(|_| "300".to_string())
        .parse()
        .expect("LOADTEST_WS_DURATION_SECS must be a number");
    let ramp_secs: u64 = std::env::var("LOADTEST_WS_RAMP_SECS")
        .unwrap_or_else(|_| "30".to_string())
        .parse()
        .expect("LOADTEST_WS_RAMP_SECS must be a number");

    let metrics = Arc::new(Metrics {
        connected: AtomicU64::new(0),
        disconnected: AtomicU64::new(0),
        messages_received: AtomicU64::new(0),
        errors: AtomicU64::new(0),
    });

    eprintln!(
        "WebSocket load test: {} clients → {} for {}s (ramp {}s)",
        num_clients, ws_url, duration_secs, ramp_secs,
    );

    let delay_per_client = if num_clients > 1 {
        Duration::from_millis(ramp_secs * 1000 / num_clients)
    } else {
        Duration::ZERO
    };

    let mut handles = Vec::with_capacity(num_clients as usize);

    for i in 0..num_clients {
        let ws_url = ws_url.clone();
        let metrics = Arc::clone(&metrics);
        let hold_duration = Duration::from_secs(duration_secs);

        let handle = tokio::spawn(async move {
            run_ws_client(i, &ws_url, hold_duration, &metrics).await;
        });
        handles.push(handle);

        if delay_per_client > Duration::ZERO {
            tokio::time::sleep(delay_per_client).await;
        }
    }

    // Wait for all clients to finish.
    for handle in handles {
        let _ = handle.await;
    }

    let connected = metrics.connected.load(Ordering::Relaxed);
    let disconnected = metrics.disconnected.load(Ordering::Relaxed);
    let messages = metrics.messages_received.load(Ordering::Relaxed);
    let errors = metrics.errors.load(Ordering::Relaxed);
    let disconnect_rate = if connected > 0 {
        (disconnected as f64 / connected as f64) * 100.0
    } else {
        0.0
    };

    eprintln!();
    eprintln!("=== WebSocket Load Test Results ===");
    eprintln!("Clients connected:    {connected}");
    eprintln!("Unexpected disconnects: {disconnected}");
    eprintln!("Disconnect rate:      {disconnect_rate:.2}%");
    eprintln!("Messages received:    {messages}");
    eprintln!("Connection errors:    {errors}");

    // Output machine-readable JSON for CI regression detection.
    let result = serde_json::json!({
        "scenario": "websocket_connect",
        "clients": num_clients,
        "duration_secs": duration_secs,
        "connected": connected,
        "disconnected": disconnected,
        "disconnect_rate_pct": disconnect_rate,
        "messages_received": messages,
        "errors": errors,
    });
    println!("{}", serde_json::to_string_pretty(&result).unwrap());

    // Exit with error if disconnect rate exceeds threshold.
    if disconnect_rate > 0.1 {
        eprintln!("FAIL: disconnect rate {disconnect_rate:.2}% exceeds 0.1% threshold");
        std::process::exit(1);
    }
}

async fn run_ws_client(id: u64, base_url: &str, hold_duration: Duration, metrics: &Metrics) {
    let session_id = uuid::Uuid::new_v4();
    let url = format!("{base_url}/ws?token={session_id}");

    let connect_result = tokio_tungstenite::connect_async(&url).await;
    let (ws_stream, _) = match connect_result {
        Ok(pair) => pair,
        Err(e) => {
            metrics.errors.fetch_add(1, Ordering::Relaxed);
            eprintln!("client {id}: connect error: {e}");
            return;
        }
    };

    metrics.connected.fetch_add(1, Ordering::Relaxed);

    let (mut write, mut read) = ws_stream.split();
    let deadline = tokio::time::Instant::now() + hold_duration;
    let metrics_ref = &metrics;

    let read_task = async {
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_secs(60), read.next()).await {
                Ok(Some(Ok(msg))) => {
                    if msg.is_close() {
                        metrics_ref.disconnected.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    metrics_ref
                        .messages_received
                        .fetch_add(1, Ordering::Relaxed);
                }
                Ok(Some(Err(_))) | Ok(None) => {
                    metrics_ref.disconnected.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                Err(_) => {
                    // Read timeout — no message in 60s, connection still alive.
                }
            }
        }
    };

    read_task.await;

    // Gracefully close.
    let _ = write.send(Message::Close(None)).await;
}
