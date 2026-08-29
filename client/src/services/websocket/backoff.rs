//! Exponential-backoff schedule for WebSocket reconnection.

/// Maximum reconnection delay in milliseconds.
const MAX_RECONNECT_DELAY_MS: u32 = 30_000;

/// Base reconnection delay in milliseconds.
const BASE_RECONNECT_DELAY_MS: u32 = 1_000;

/// Delay before reconnect attempt number `attempts` (0-based).
///
/// Exponential backoff: `base * 2^attempts`, capped at
/// [`MAX_RECONNECT_DELAY_MS`].
pub(super) fn reconnect_delay_ms(attempts: u32) -> u32 {
    BASE_RECONNECT_DELAY_MS
        .saturating_mul(2u32.saturating_pow(attempts))
        .min(MAX_RECONNECT_DELAY_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reconnect_delay_calculation() {
        // Verify the exponential backoff formula
        assert_eq!(reconnect_delay_ms(0), 1_000);
        assert_eq!(reconnect_delay_ms(1), 2_000);
        assert_eq!(reconnect_delay_ms(2), 4_000);
        assert_eq!(reconnect_delay_ms(3), 8_000);
        assert_eq!(reconnect_delay_ms(4), 16_000);
        assert_eq!(reconnect_delay_ms(5), 30_000);
        assert_eq!(reconnect_delay_ms(6), 30_000);
    }

    #[test]
    fn test_reconnect_delay_saturates_at_max() {
        assert_eq!(reconnect_delay_ms(u32::MAX), 30_000);
    }
}
