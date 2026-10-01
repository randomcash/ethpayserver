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

use crate::error::{EvmError, EvmResult};

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

/// Build one monitor per chain id, each under [`retry_startup`]. The first
/// chain that still fails after its retries aborts the whole startup with an
/// error naming that chain: a process missing a monitor must not run, because
/// nothing would tell anyone it detects no payments there.
pub async fn build_monitors<M, F, Fut>(
    chain_ids: &[u64],
    attempts: u32,
    initial_delay: Duration,
    mut create: F,
) -> EvmResult<Vec<(u64, M)>>
where
    F: FnMut(u64) -> Fut,
    Fut: Future<Output = EvmResult<M>>,
{
    let mut monitors = Vec::with_capacity(chain_ids.len());
    for &chain_id in chain_ids {
        let monitor = retry_startup("create chain monitor", attempts, initial_delay, || {
            create(chain_id)
        })
        .await
        .map_err(|e| {
            tracing::error!(chain_id, error = %e, "failed to create chain monitor");
            EvmError::Monitor(format!(
                "failed to create chain monitor for chain {chain_id}: {e}"
            ))
        })?;
        monitors.push((chain_id, monitor));
    }
    Ok(monitors)
}

#[cfg(test)]
mod tests {
    use super::*;
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

    /// The defect: a chain whose monitor cannot be built must fail startup,
    /// not be logged and skipped. Replacing the `?` with a log-and-continue
    /// turns this red.
    #[tokio::test(start_paused = true)]
    async fn persistent_failure_fails_startup_naming_the_chain() {
        let out: EvmResult<Vec<(u64, u8)>> =
            build_monitors(&[1, 11155111], 3, Duration::from_secs(1), |id| async move {
                if id == 11155111 {
                    Err(EvmError::Monitor("failed to get chain ID".into()))
                } else {
                    Ok(0u8)
                }
            })
            .await;
        let err = out.unwrap_err().to_string();
        assert!(err.contains("11155111"), "{err}");
    }

    #[tokio::test(start_paused = true)]
    async fn transient_failure_still_yields_every_monitor() {
        let calls = AtomicU32::new(0);
        let out = build_monitors(&[1, 2], 3, Duration::from_secs(1), |id| {
            let first = calls.fetch_add(1, Ordering::SeqCst) == 0;
            async move {
                if first {
                    Err(EvmError::Monitor("blip".into()))
                } else {
                    Ok(id)
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(out, vec![(1, 1), (2, 2)]);
    }
}
