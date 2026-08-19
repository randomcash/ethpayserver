//! Tunables for the idempotency middleware.

/// Maximum length for idempotency key values.
pub(super) const MAX_KEY_LENGTH: usize = 255;

/// Maximum request body size to buffer (1 MiB).
///
/// Invoice create payloads are small (hundreds of bytes), so 1 MiB is a
/// generous ceiling while keeping the pre-handler DoS surface bounded —
/// callers sending larger bodies with an idempotency key get 413.
pub(super) const MAX_REQUEST_BODY: usize = 1024 * 1024;

/// Maximum response body size to cache (1 MiB).
///
/// Well above expected JSON response size. Larger responses bypass the
/// cache (logged) rather than eating Redis memory.
pub(super) const MAX_RESPONSE_BODY: usize = 1024 * 1024;

/// Lock TTL for in-flight requests (5 minutes).
///
/// Must be longer than the worst-case handler latency (DB contention,
/// address derivation, chain RPC). If the lock expires before the handler
/// finishes, a duplicate retry would bypass idempotency — so err on the
/// side of holding the lock too long rather than too short. Stale locks
/// self-clear via the TTL once the original handler returns.
pub(super) const LOCK_TTL_SECS: u64 = 300;

/// Redis key prefix for idempotency cache entries.
pub(super) const CACHE_PREFIX: &str = "idem";

/// Redis key prefix for in-flight locks.
pub(super) const LOCK_PREFIX: &str = "idem_lock";

/// Default cache TTL: 24 hours.
pub(super) const DEFAULT_TTL_SECS: u64 = 86400;
