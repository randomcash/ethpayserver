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
use rand::Rng;
use redis::aio::ConnectionManager;
use redis::streams::{StreamRangeReply, StreamReadOptions, StreamReadReply};
use redis::{AsyncCommands, Client, Script};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio_stream::StreamExt;
use tracing::{debug, error, info, warn};

/// `(epoch, seq, chain_id, block_height, payload)` of an outbox entry, or
/// `None` if any field is missing or unparseable.
fn parse_entry(entry: &redis::streams::StreamId) -> Option<(i64, i64, u64, i64, String)> {
    let get_field = |name: &str| -> Option<String> {
        entry
            .map
            .get(name)
            .and_then(|v| redis::from_redis_value::<String>(v).ok())
    };
    Some((
        get_field("epoch")?.parse().ok()?,
        get_field("seq")?.parse().ok()?,
        get_field("chain_id")?.parse().ok()?,
        get_field("block_height")?.parse().ok()?,
        get_field("payload")?,
    ))
}

/// Whether the outbox's epoch key still holds `expected`. A keyspace loss
/// while subscribed resets `seq` to 1, so every new entry sorts below the
/// reader's `last_id` and `XREAD` blocks forever without erroring. Checked
/// before every `XREAD` (which returns at least every 5s); on "no" the
/// stream ends so `run` re-enters the startup lineage check. A read error
/// counts as "no": ending the stream is the safe answer to not knowing.
async fn epoch_unchanged(
    conn: &mut redis::aio::MultiplexedConnection,
    epoch_key: &str,
    expected: i64,
) -> bool {
    let current: Result<Option<String>, _> = conn.get(epoch_key).await;
    matches!(current, Ok(Some(v)) if v.parse::<i64>().ok() == Some(expected))
}

/// Mint a fresh epoch identity.
///
/// Drawn from the full positive range of `i64`, not a counter: a counter
/// needs its own durable storage to guarantee it never repeats a value it
/// has already issued, but here that storage would have to live in the same
/// Redis instance as the epoch key itself, so a total keyspace loss (a
/// restart with no AOF/RDB, an evicted keyspace) resets the counter right
/// alongside the value it exists to keep unique - the very lineage break
/// this mechanism must detect. A random draw from a ~9.2*10^18 range makes a
/// collision with any specific prior epoch negligibly improbable regardless
/// of what the backing store does or does not remember, with no auxiliary
/// key to lose.
fn random_epoch() -> i64 {
    rand::rng().random_range(1..=i64::MAX)
}

/// Whether a subscription stream ending is a fault worth reporting.
///
/// Split out from the two stream tails below so the shutdown/fault decision
/// itself is unit-testable without a live redis connection.
fn subscription_end_is_fault(shutting_down: bool) -> bool {
    !shutting_down
}

/// Whether a freshly read entry continues cleanly from `expected_seq`,
/// logging a detected gap before reporting it.
///
/// Split out of `subscribe_from`'s stream body so the mid-batch check - not
/// just the once-before-the-loop retention check above it - can run on
/// every entry without pushing that function over the line-count lint.
/// `expected_seq` is `None` only before the first entry, when any starting
/// point is valid.
fn is_seq_gap(stream_key: &str, expected_seq: Option<i64>, seq: i64) -> bool {
    let Some(expected) = expected_seq else {
        return false;
    };
    if seq == expected {
        return false;
    }
    error!(
        expected_seq = expected,
        seq,
        stream = %stream_key,
        "event stream gap detected mid-subscription; ending the stream rather than silently \
         skipping the trimmed entries"
    );
    true
}

/// Log a subscription stream ending at the level its cause deserves.
///
/// `kind` names the stream ("events" or "commands") for the log message.
/// Factored out of the two stream tails below so it's directly testable
/// under a captured subscriber, rather than only through the extracted
/// `subscription_end_is_fault` boolean.
fn log_subscription_end(kind: &str, channel: &str, shutting_down: bool) {
    if subscription_end_is_fault(shutting_down) {
        error!(channel = %channel, "redis {} subscription ended unexpectedly", kind);
    } else {
        info!(
            channel = %channel,
            "redis {} subscription ended: shutdown in progress", kind
        );
    }
}

/// Cap on retained stream entries. Approximate (`~`) trimming is O(1) per
/// `XADD` rather than an exact trim's O(log n), and a consumer that falls
/// this far behind needs the same loud "I cannot resume" fallback as one
/// whose epoch changed - trying to save the last few thousand entries near
/// the boundary buys nothing. `subscribe_from` enforces the fallback; this
/// constant only bounds how much a resumer can fall behind before it fires.
const STREAM_MAXLEN: usize = 200_000;

/// Allocates `seq`, reads (or creates) the current epoch, and appends the
/// entry under both in one atomic step.
///
/// A plain `INCR` followed by a separate `XADD <id>` round trip lets two
/// concurrent publishers interleave: whichever `XADD` lands second at the
/// server can carry the *smaller* `seq`, if that process paused between its
/// own `INCR` and `XADD`. Stream entry IDs must be strictly increasing at
/// the server, so Redis rejects that `XADD` outright - and because nothing
/// durable was ever written for it, the event is gone with no cursor gap to
/// detect and no replay path to recover it. The epoch has the same problem
/// one level up: reading it in a separate round trip before this script
/// leaves a window where `bump_epoch` lands in between, so the entry is
/// stamped with an epoch that is already stale by the time it is written.
/// Running all three steps inside one script closes both: Redis executes a
/// script as a single atomic unit, so no other client's commands - not even
/// another invocation of this same script, nor a concurrent `bump_epoch` -
/// can interleave with the `INCR`, the epoch read, and the `XADD` it feeds.
///
/// If nobody has minted an epoch for this outbox yet, the caller draws one
/// with [`random_epoch`] (see its doc comment for why: it survives a total
/// Redis keyspace loss that a same-instance counter cannot) and passes it in
/// as `ARGV[5]`; `SET ... NX` inside the script is what makes that draw safe
/// against a concurrent publisher doing the same thing (see below).
const PUBLISH_SCRIPT: &str = r"
    local seq = redis.call('INCR', KEYS[2])
    local epoch = redis.call('GET', KEYS[3])
    if not epoch then
        redis.call('SET', KEYS[3], ARGV[5], 'NX')
        epoch = redis.call('GET', KEYS[3])
    end
    redis.call('XADD', KEYS[1], 'MAXLEN', '~', ARGV[1], seq .. '-0',
        'epoch', epoch, 'seq', seq, 'chain_id', ARGV[2],
        'block_height', ARGV[3], 'payload', ARGV[4])
    return {seq, epoch}
";

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
    /// Cap on retained stream entries. See [`STREAM_MAXLEN`]; only
    /// overridden by [`RedisBridge::new_with_maxlen`], which exists so a
    /// test can force a retention gap without publishing 200,000 entries.
    maxlen: usize,
    /// Set once the owning process has asked to stop.
    ///
    /// A subscription stream ending is only a fault when nobody asked it to;
    /// during a normal shutdown the surrounding container's network can drop
    /// out from under it, which looks identical at the redis client level.
    shutting_down: Arc<AtomicBool>,
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
            maxlen: STREAM_MAXLEN,
            shutting_down: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Create a new Redis bridge with a non-default retention cap.
    ///
    /// Test-only in practice: production deployments want [`STREAM_MAXLEN`],
    /// and this exists so a test can force a retention gap by publishing a
    /// handful of entries against a small `maxlen` rather than 200,000
    /// against the real one.
    pub async fn new_with_maxlen(
        url: &str,
        events_channel: &str,
        commands_channel: &str,
        maxlen: usize,
    ) -> EvmResult<Self> {
        let mut bridge = Self::new(url, events_channel, commands_channel).await?;
        bridge.maxlen = maxlen;
        Ok(bridge)
    }

    /// Mark this bridge as shutting down intentionally.
    ///
    /// Call this from the process's own shutdown handler, before tearing
    /// anything else down. A subscription stream that ends afterwards logs
    /// at `info` instead of `error` — it ended because we asked it to, not
    /// because something broke.
    pub fn begin_shutdown(&self) {
        self.shutting_down.store(true, Ordering::Relaxed);
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
    /// `SET ... NX` races safely: if two callers lose the key at once, both
    /// draw their own candidate from [`random_epoch`], only one write
    /// sticks, and the follow-up `GET` returns whichever won for both of
    /// them.
    async fn get_or_init_epoch(&self) -> EvmResult<i64> {
        let mut conn = self.publisher.clone();
        let key = self.epoch_key();

        if let Some(epoch) = self.get_key(&key).await? {
            return epoch
                .parse()
                .map_err(|e| EvmError::Monitor(format!("corrupt epoch value {epoch:?}: {e}")));
        }

        let candidate = random_epoch();
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

    /// The `seq` of the oldest entry this outbox still retains, or `None` if
    /// it currently has none at all (nothing published yet, or trimmed down
    /// to nothing).
    async fn oldest_retained_seq(&self) -> EvmResult<Option<i64>> {
        let mut conn = self.publisher.clone();
        let reply: StreamRangeReply = conn
            .xrange_count(&self.events_channel, "-", "+", 1)
            .await
            .map_err(|e| EvmError::Monitor(format!("redis XRANGE failed: {}", e)))?;

        // An id that does not parse must fail, not read as "nothing
        // retained": that would switch the out-of-range guard off.
        reply
            .ids
            .first()
            .map(|entry| {
                entry
                    .id
                    .split('-')
                    .next()
                    .and_then(|seq| seq.parse().ok())
                    .ok_or_else(|| {
                        EvmError::Monitor(format!("unparseable stream entry id {:?}", entry.id))
                    })
            })
            .transpose()
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

        let chain_id = event.chain_id();
        let block_height = event.block_height();

        // Drawn even on the common path where the epoch key already exists
        // and this goes unused: minting it inside the script would need a
        // second Lua RNG call whose determinism across replication is not
        // worth relying on, and one wasted `random_range` call is cheap.
        let candidate_epoch = random_epoch();

        let mut conn = self.publisher.clone();
        let (seq, epoch): (i64, i64) = Script::new(PUBLISH_SCRIPT)
            .key(&self.events_channel)
            .key(self.seq_key())
            .key(self.epoch_key())
            .arg(self.maxlen)
            .arg(chain_id)
            .arg(block_height)
            .arg(payload)
            .arg(candidate_epoch)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| EvmError::Monitor(format!("redis publish script failed: {}", e)))?;

        debug!(stream = %self.events_channel, seq, epoch, "published event to redis stream");
        Ok(())
    }

    async fn subscribe_from(&self, from: Option<EventCursor>) -> EvmResult<DurableEventStream> {
        // Same epoch does not, on its own, mean `cursor.seq` is still safe
        // to resume from: `XADD ... MAXLEN` trims independently of the
        // epoch key, so a consumer that falls behind the retention window
        // can have its committed position trimmed out while the epoch never
        // moved. Left unchecked, the `XREAD` below would silently resume
        // from whatever the stream happens to retain next - exactly the
        // "starting from wherever" failure this whole mechanism exists to
        // rule out. The epoch is left alone on purpose: bumping it would make
        // the next start read a mismatch and resume past the gap silently,
        // so instead every restart fails until an operator audits the gap.
        // Read once, before anything else, and compared to the cursor's
        // epoch: the caller read the epoch separately, so the keyspace can
        // have been lost in between. `seq` numbers from another lineage
        // must not be used as an `XREAD` position - they would skip or hang.
        let subscribed_epoch = self.get_or_init_epoch().await?;
        if let Some(cursor) = from
            && cursor.epoch != subscribed_epoch
        {
            return Err(EvmError::EventStreamOutOfRange(format!(
                "resume cursor names epoch {} but the outbox is at epoch {subscribed_epoch}",
                cursor.epoch
            )));
        }

        if let Some(cursor) = from
            && let Some(oldest) = self.oldest_retained_seq().await?
            && oldest > cursor.seq + 1
        {
            return Err(EvmError::EventStreamOutOfRange(format!(
                "resume at seq {} is behind the oldest retained entry (seq {oldest})",
                cursor.seq
            )));
        }

        let client = self.client.clone();
        let stream_key = self.events_channel.clone();
        let shutting_down = Arc::clone(&self.shutting_down);
        // XREAD returns IDs strictly greater than the one given; entry IDs
        // are `{seq}-0`, so after `{cursor.seq}-0` is "replay from seq + 1"
        // and after `0-0` is "replay everything retained".
        let mut last_id = match from {
            Some(cursor) => format!("{}-0", cursor.seq),
            None => "0-0".to_string(),
        };
        // The check above runs once; `is_seq_gap` covers later batches.
        let mut expected_seq = from.map(|cursor| cursor.seq + 1);

        // See `epoch_unchanged` for why the epoch is re-checked while subscribed.
        let epoch_key = self.epoch_key();

        let s = stream! {
            let mut conn = match client.get_multiplexed_async_connection().await {
                Ok(c) => c,
                Err(e) => {
                    error!(error = %e, "redis stream connection failed");
                    return;
                }
            };

            loop {
                if !epoch_unchanged(&mut conn, &epoch_key, subscribed_epoch).await {
                    error!(stream = %stream_key, subscribed_epoch, "outbox epoch changed or vanished while subscribed; ending the stream");
                    return;
                }

                let opts = StreamReadOptions::default().count(500).block(5_000);
                let reply: StreamReadReply = match conn
                    .xread_options(&[stream_key.as_str()], &[last_id.as_str()], &opts)
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        // The surrounding container's network can drop out
                        // from under this connection while it's being torn
                        // down, which looks identical to a real fault here -
                        // `shutting_down` (set by the owning process's
                        // shutdown handler before it tears anything else
                        // down) is what tells the two apart.
                        debug!(error = %e, stream = %stream_key, "redis XREAD ended");
                        log_subscription_end("events", &stream_key, shutting_down.load(Ordering::Relaxed));
                        return;
                    }
                };

                for key in reply.keys {
                    for entry in key.ids {
                        last_id = entry.id.clone();

                        // `last_id` above already advanced past this entry, so a
                        // plain `continue` here would move on for good: no cursor
                        // gap to detect, no replay path to recover it - the same
                        // permanent loss this whole mechanism exists to close,
                        // just reached through a corrupt entry instead of a
                        // crash. Ending the stream instead lets `run` react the
                        // same way it does to any other fatal resume condition
                        // (a dead connection, a failed `XREAD`): the consumer
                        // halts rather than silently skipping a payment.
                        let Some((epoch, seq, chain_id, block_height, payload)) = parse_entry(&entry) else {
                            error!(id = %entry.id, "malformed stream entry; ending the stream rather than skipping it");
                            return;
                        };

                        // Ending here sends the consumer back through
                        // `subscribe_from`, whose retention check above
                        // turns this into the same `EventStreamOutOfRange`
                        // a resume-time gap gets.
                        if is_seq_gap(&stream_key, expected_seq, seq) {
                            return;
                        }
                        expected_seq = Some(seq + 1);

                        match serde_json::from_str::<MonitorEvent>(&payload) {
                            Ok(event) => yield EventEnvelope {
                                chain_id,
                                cursor: EventCursor { epoch, seq, block_height },
                                event,
                            },
                            Err(e) => {
                                error!(error = %e, payload = %payload, "failed to deserialize event; ending the stream rather than skipping it");
                                return;
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

    async fn bump_epoch(&self) -> EvmResult<i64> {
        let mut conn = self.publisher.clone();
        let new_epoch = random_epoch();
        // Unconditional SET, not `NX`: `get_or_init_epoch` uses `NX` because
        // it must not clobber a value another caller already agreed on, but
        // this is the one call whose entire job is to make every existing
        // agreement stale.
        let _: () = conn
            .set(self.epoch_key(), new_epoch)
            .await
            .map_err(|e| EvmError::Monitor(format!("redis SET failed: {}", e)))?;
        Ok(new_epoch)
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
        let shutting_down = Arc::clone(&self.shutting_down);
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
            log_subscription_end("commands", &channel, shutting_down.load(Ordering::Relaxed));
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

    #[test]
    fn subscription_end_during_shutdown_is_not_a_fault() {
        assert!(!subscription_end_is_fault(true));
    }

    #[test]
    fn subscription_end_without_shutdown_is_a_fault() {
        assert!(subscription_end_is_fault(false));
    }

    /// Runs `log_subscription_end` under a subscriber that captures its
    /// output, so the tests below exercise the real `error!`/`info!` call
    /// sites the stream tails use — not just the extracted
    /// `subscription_end_is_fault` boolean.
    fn capture_log_subscription_end(shutting_down: bool) -> String {
        use std::io;
        use std::sync::Mutex;

        #[derive(Clone, Default)]
        struct Buf(Arc<Mutex<Vec<u8>>>);

        impl io::Write for Buf {
            fn write(&mut self, data: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buf {
            type Writer = Buf;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let buf = Buf::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_ansi(false)
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            log_subscription_end("events", "test-channel", shutting_down);
        });

        String::from_utf8(buf.0.lock().unwrap().clone()).expect("utf8 log output")
    }

    #[test]
    fn log_subscription_end_reports_error_when_not_shutting_down() {
        let output = capture_log_subscription_end(false);
        assert!(output.contains("ERROR"), "expected ERROR, got: {output}");
    }

    #[test]
    fn log_subscription_end_reports_info_when_shutting_down() {
        let output = capture_log_subscription_end(true);
        assert!(output.contains("INFO"), "expected INFO, got: {output}");
        assert!(
            !output.contains("ERROR"),
            "shutdown noise must not reach error level: {output}"
        );
    }
}
