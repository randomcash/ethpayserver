//! Periodic health publishing to Redis.

use std::sync::Arc;

use evm::monitor::{MonitorCoordinator, RpcBlockSource};
use tracing::{debug, error, info, warn};

/// Redis key for publishing health information.
const HEALTH_KEY: &str = "evmmonitor:health";

/// Periodically publish health information to Redis.
///
/// Health info is stored as JSON in the HEALTH_KEY with a 60-second TTL.
/// This allows the API server to read health without direct communication.
pub(crate) async fn publish_health_loop(
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
