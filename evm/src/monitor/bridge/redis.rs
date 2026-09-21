//! Redis event bridge: a durable stream for events, pub/sub for commands.
//!
//! - **Events channel**: monitor publishes to a Redis Stream (durable,
//!   replayable); API servers resume it from a cursor. A dropped API server
//!   does not lose what was published while it was gone - the reason this
//!   is a stream and not `PUBLISH`/`SUBSCRIBE`.
//! - **Commands channel**: API servers publish, monitors subscribe, over
//!   plain pub/sub. `watch_retry` already re-drives a lost command within
//!   30 seconds, so this side does not need the same durability.
//!
//! Supports multiple monitors publishing to the same stream and multiple
//! API servers resuming it independently.

use super::{CommandStream, DurableEventStream, EventBridge, EventCursor, EventEnvelope};
use crate::error::{EvmError, EvmResult};
use crate::monitor::events::{MonitorCommand, MonitorEvent};
use async_stream::stream;
use async_trait::async_trait;
use redis::aio::ConnectionManager;
use redis::streams::{StreamMaxlen, StreamReadOptions, StreamReadReply};
use redis::{AsyncCommands, Client};
use tokio_stream::StreamExt;
use tracing::{debug, error, warn};

/// Cap on retained stream entries. Approximate (`~`) trimming is O(1) per
/// `XADD` rather than an exact trim's O(log n), and a consumer that falls
/// this far behind needs the same loud "I cannot resume" fallback as one
/// whose epoch changed - trying to save the last few thousand entries near
/// the boundary buys nothing.
const STREAM_MAXLEN: usize = 200_000;

/// Redis event bridge.
pub struct RedisBridge {
    /// Redis client for creating connections.
    client: Client,
    /// Connection manager for publishing (connection pool).
    publisher: ConnectionManager,
    /// Stream key for events (monitor -> API server).
    events_channel: String,
    /// Channel name for commands (API server -> monitor).
    commands_channel: String,
}

impl RedisBridge {
    /// Create a new Redis bridge.
    ///
    /// # Arguments
    ///
    /// * `url` - Redis connection URL (e.g., "redis://localhost:6379")
    /// * `events_channel` - Stream key for events (e.g., "evmmonitor:events")
    /// * `commands_channel` - Channel for commands (e.g., "evmmonitor:commands")
    pub async fn new(url: &str, events_channel: &str, commands_channel: &str) -> EvmResult<Self> {
        let client = Client::open(url)
            .map_err(|e| EvmError::Monitor(format!("redis connection failed: {}", e)))?;

        let publisher = ConnectionManager::new(client.clone())
            .await
            .map_err(|e| EvmError::Monitor(format!("redis connection manager failed: {}", e)))?;

        Ok(Self {
            client,
            publisher,
            events_channel: events_channel.to_string(),
            commands_channel: commands_channel.to_string(),
        })
    }

    /// Get the events stream key.
    pub fn events_channel(&self) -> &str {
        &self.events_channel
    }

    /// Get the commands channel name.
    pub fn commands_channel(&self) -> &str {
        &self.commands_channel
    }

    /// Get a value from Redis by key.
    pub async fn get_key(&self, key: &str) -> EvmResult<Option<String>> {
        let mut conn = self.publisher.clone();
        let value: Option<String> = conn
            .get(key)
            .await
            .map_err(|e| EvmError::Monitor(format!("redis GET failed: {}", e)))?;
        Ok(value)
    }

    /// Key holding this outbox's current epoch.
    fn epoch_key(&self) -> String {
        format!("{}:epoch", self.events_channel)
    }

    /// Key of the counter this outbox draws `seq` from.
    fn seq_key(&self) -> String {
        format!("{}:seq", self.events_channel)
    }

    /// The outbox's epoch, creating one if this is the first publisher or
    /// resumer to ever see this outbox (a fresh deployment, or a Redis that
    /// lost the key along with everything else).
    ///
    /// `SET ... NX` races safely: if two callers lose the key at once, only
    /// one write sticks, and the follow-up `GET` returns whichever won for
    /// both of them.
    async fn get_or_init_epoch(&self) -> EvmResult<i64> {
        let mut conn = self.publisher.clone();
        let key = self.epoch_key();

        if let Some(epoch) = self.get_key(&key).await? {
            return epoch
                .parse()
                .map_err(|e| EvmError::Monitor(format!("corrupt epoch value {epoch:?}: {e}")));
        }

        let candidate = chrono::Utc::now().timestamp_millis();
        let _: () = redis::cmd("SET")
            .arg(&key)
            .arg(candidate)
            .arg("NX")
            .query_async(&mut conn)
            .await
            .map_err(|e| EvmError::Monitor(format!("redis SET NX failed: {}", e)))?;

        let epoch: String = conn
            .get(&key)
            .await
            .map_err(|e| EvmError::Monitor(format!("redis GET failed: {}", e)))?;
        epoch
            .parse()
            .map_err(|e| EvmError::Monitor(format!("corrupt epoch value {epoch:?}: {e}")))
    }
}

#[async_trait]
impl EventBridge for RedisBridge {
    // =========================================================================
    // Events (Monitor -> API Server)
    // =========================================================================

    async fn publish(&self, event: &MonitorEvent) -> EvmResult<()> {
        let payload = serde_json::to_string(event)
            .map_err(|e| EvmError::Monitor(format!("event serialization failed: {}", e)))?;

        let epoch = self.get_or_init_epoch().await?;

        let mut conn = self.publisher.clone();
        let seq: i64 = conn
            .incr(self.seq_key(), 1)
            .await
            .map_err(|e| EvmError::Monitor(format!("redis INCR failed: {}", e)))?;

        // Our own seq drives the entry's ID, so resuming "after seq" is a
        // plain ID-range read - no separate index from our seq to Redis's
        // own ID is needed. This is safe only because `seq` is assigned by
        // one atomic INCR immediately above: IDs handed to XADD must be
        // strictly increasing, and a monotonic counter with no gaps
        // guarantees that.
        let id = format!("{seq}-0");
        let chain_id = event.chain_id();
        let block_height = event.block_height();

        let _: String = conn
            .xadd_maxlen(
                &self.events_channel,
                StreamMaxlen::Approx(STREAM_MAXLEN),
                &id,
                &[
                    ("epoch", epoch.to_string()),
                    ("seq", seq.to_string()),
                    ("chain_id", chain_id.to_string()),
                    ("block_height", block_height.to_string()),
                    ("payload", payload),
                ],
            )
            .await
            .map_err(|e| EvmError::Monitor(format!("redis XADD failed: {}", e)))?;

        debug!(stream = %self.events_channel, seq, epoch, "published event to redis stream");
        Ok(())
    }

    async fn subscribe_from(&self, from: Option<EventCursor>) -> EvmResult<DurableEventStream> {
        let client = self.client.clone();
        let stream_key = self.events_channel.clone();
        // XREAD returns entries with an ID strictly greater than the one
        // given. Our entry IDs are always `{seq}-0`, so asking for anything
        // after `{cursor.seq}-0` is exactly "replay from seq + 1"; asking
        // for anything after `0-0` (nothing published ever has that ID) is
        // "replay everything retained".
        let mut last_id = match from {
            Some(cursor) => format!("{}-0", cursor.seq),
            None => "0-0".to_string(),
        };

        let s = stream! {
            let mut conn = match client.get_multiplexed_async_connection().await {
                Ok(c) => c,
                Err(e) => {
                    error!(error = %e, "redis stream connection failed");
                    return;
                }
            };

            loop {
                let opts = StreamReadOptions::default().count(500).block(5_000);
                let reply: StreamReadReply = match conn
                    .xread_options(&[stream_key.as_str()], &[last_id.as_str()], &opts)
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        error!(error = %e, stream = %stream_key, "redis XREAD failed");
                        return;
                    }
                };

                for key in reply.keys {
                    for entry in key.ids {
                        last_id = entry.id.clone();

                        let get_field = |name: &str| -> Option<String> {
                            entry
                                .map
                                .get(name)
                                .and_then(|v| redis::from_redis_value::<String>(v).ok())
                        };

                        let (Some(epoch), Some(seq), Some(chain_id), Some(block_height), Some(payload)) = (
                            get_field("epoch").and_then(|v| v.parse().ok()),
                            get_field("seq").and_then(|v| v.parse().ok()),
                            get_field("chain_id").and_then(|v| v.parse().ok()),
                            get_field("block_height").and_then(|v| v.parse().ok()),
                            get_field("payload"),
                        ) else {
                            warn!(id = %entry.id, "malformed stream entry, skipping");
                            continue;
                        };

                        match serde_json::from_str::<MonitorEvent>(&payload) {
                            Ok(event) => yield EventEnvelope {
                                chain_id,
                                cursor: EventCursor { epoch, seq, block_height },
                                event,
                            },
                            Err(e) => {
                                warn!(error = %e, payload = %payload, "failed to deserialize event");
                            }
                        }
                    }
                }
            }
        };

        Ok(Box::pin(s))
    }

    async fn current_epoch(&self) -> EvmResult<i64> {
        self.get_or_init_epoch().await
    }

    // =========================================================================
    // Commands (API Server -> Monitor)
    // =========================================================================

    async fn publish_command(&self, command: &MonitorCommand) -> EvmResult<()> {
        let payload = serde_json::to_string(command)
            .map_err(|e| EvmError::Monitor(format!("command serialization failed: {}", e)))?;

        let mut conn = self.publisher.clone();
        conn.publish::<_, _, ()>(&self.commands_channel, &payload)
            .await
            .map_err(|e| EvmError::Monitor(format!("redis publish command failed: {}", e)))?;

        debug!(channel = %self.commands_channel, "published command to redis");
        Ok(())
    }

    async fn subscribe_commands(&self) -> EvmResult<CommandStream> {
        let mut pubsub = self
            .client
            .get_async_pubsub()
            .await
            .map_err(|e| EvmError::Monitor(format!("redis pubsub failed: {}", e)))?;

        pubsub
            .subscribe(&self.commands_channel)
            .await
            .map_err(|e| EvmError::Monitor(format!("redis subscribe commands failed: {}", e)))?;

        let channel = self.commands_channel.clone();
        let stream = async_stream::stream! {
            let mut msg_stream = pubsub.on_message();
            while let Some(msg) = msg_stream.next().await {
                let payload: String = match msg.get_payload() {
                    Ok(p) => p,
                    Err(e) => {
                        warn!(error = %e, "failed to get redis command payload");
                        continue;
                    }
                };

                match serde_json::from_str::<MonitorCommand>(&payload) {
                    Ok(command) => {
                        debug!(channel = %channel, "received command from redis");
                        yield command;
                    }
                    Err(e) => {
                        warn!(error = %e, payload = %payload, "failed to deserialize command");
                    }
                }
            }
            error!(channel = %channel, "redis commands subscription ended unexpectedly");
        };

        Ok(Box::pin(stream))
    }

    // =========================================================================
    // Utility
    // =========================================================================

    fn name(&self) -> &str {
        "RedisBridge"
    }

    async fn health_check(&self) -> EvmResult<()> {
        let mut conn = self.publisher.clone();
        let pong: String = redis::cmd("PING")
            .query_async(&mut conn)
            .await
            .map_err(|e| EvmError::Monitor(format!("redis health check failed: {}", e)))?;

        if pong == "PONG" {
            Ok(())
        } else {
            Err(EvmError::Monitor(
                "redis health check: unexpected response".to_string(),
            ))
        }
    }
}

// Note: Integration tests for Redis require a running Redis instance.
// These are typically run in CI with a Redis service or skipped locally.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redis_url_parsing() {
        // Just verify URL parsing works
        let result = Client::open("redis://localhost:6379");
        assert!(result.is_ok());
    }
}
