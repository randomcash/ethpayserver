//! Bounded retry for startup work that talks to a remote RPC endpoint.
//!
//! A single failed request at process start is routine (a provider blip, a
//! DNS hiccup). Giving up on the first one leaves a process that is alive but
//! cannot do its job, and nothing restarts it. Retrying forever is no better:
//! a wrong URL or a revoked key would spin silently. So: a few attempts with
//! growing delays, then the last error is returned for the caller to treat as
//! fatal and let the supervisor restart the process.

use std::future::Future;
use std::time::Duration;

use crate::error::EvmResult;

/// Total attempts, including the first.
pub const STARTUP_ATTEMPTS: u32 = 5;

/// Delay after the first failure; doubles after each further failure.
pub const STARTUP_INITIAL_DELAY: Duration = Duration::from_secs(2);

/// Run `op` until it succeeds or `attempts` have been used, sleeping
/// `initial_delay`, then double that, between tries. Returns the last error
/// if every attempt fails.
pub async fn retry_startup<T, F, Fut>(
    what: &str,
    attempts: u32,
    initial_delay: Duration,
    mut op: F,
) -> EvmResult<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = EvmResult<T>>,
{
    let attempts = attempts.max(1);
    let mut delay = initial_delay;
    let mut attempt = 1;
    loop {
        match op().await {
            Ok(value) => return Ok(value),
            Err(e) if attempt >= attempts => return Err(e),
            Err(e) => {
                tracing::warn!(
                    what,
                    attempt,
                    attempts,
                    retry_in = ?delay,
                    error = %e,
                    "startup step failed, retrying"
                );
                tokio::time::sleep(delay).await;
                delay = delay.saturating_mul(2);
                attempt += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::EvmError;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test(start_paused = true)]
    async fn recovers_after_transient_failures() {
        let calls = AtomicU32::new(0);
        let out = retry_startup("t", 5, Duration::from_secs(1), || async {
            if calls.fetch_add(1, Ordering::SeqCst) < 2 {
                Err(EvmError::Monitor("blip".into()))
            } else {
                Ok(7)
            }
        })
        .await;
        assert_eq!(out.unwrap(), 7);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn persistent_failure_is_bounded_and_returned() {
        let calls = AtomicU32::new(0);
        let out: EvmResult<()> = retry_startup("t", 4, Duration::from_secs(1), || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(EvmError::Monitor("down".into()))
        })
        .await;
        assert!(out.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn delay_doubles_between_attempts() {
        let start = tokio::time::Instant::now();
        let _: EvmResult<()> = retry_startup("t", 4, Duration::from_secs(1), || async {
            Err(EvmError::Monitor("down".into()))
        })
        .await;
        // 1 + 2 + 4 between four attempts, none after the last.
        assert_eq!(start.elapsed(), Duration::from_secs(7));
    }
}
