//! Pure time-window predicates `extractors` enforces at request time.
//!
//! Split out of `extractors` - `is_reauth_stale` and `is_grace_expired` have
//! no axum/request dependency of their own, and their unit tests were a
//! large share of that file's size.

use chrono::{DateTime, Utc};

/// How long ago a session must have been created to count as a fresh proof of
/// a passkey or wallet assertion. Matches the window `cleanup_expired_challenges`
/// (auth crate, wallet/passkey challenge tables) treats a login challenge as
/// live for, so "recent enough to prove you just authenticated" means the same
/// thing everywhere in this codebase.
const REAUTH_FRESHNESS: chrono::Duration = chrono::Duration::minutes(5);

/// Pure predicate: is a session's login assertion too old to count as a
/// fresh re-authentication at `now`?
///
/// Extracted so the freshness window `FreshlyAuthenticatedUser` enforces can
/// be unit-tested without booting a database or a `SessionService`. Matches
/// the live check in `FreshlyAuthenticatedUser::from_request_parts` exactly.
pub(super) fn is_reauth_stale(session_created_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    now - session_created_at > REAUTH_FRESHNESS
}

/// Pure predicate: is a deprecated key past its grace window at `now`?
///
/// Extracted so the grace-expiry rule can be unit-tested without booting a
/// database or touching the extractor wiring. Matches the live check in
/// `validate_api_key` exactly.
pub(super) fn is_grace_expired(
    deprecated_at: DateTime<Utc>,
    now: DateTime<Utc>,
    grace_secs: i64,
) -> bool {
    now > deprecated_at + chrono::Duration::seconds(grace_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn at(hour: i64) -> DateTime<Utc> {
        // Fixed base date so tests are deterministic; chrono's Utc::now drift
        // would otherwise race the grace-window arithmetic.
        Utc.with_ymd_and_hms(2026, 4, 23, 0, 0, 0).unwrap() + Duration::hours(hour)
    }

    const GRACE_48H: i64 = 48 * 3600;

    #[test]
    fn fresh_session_is_not_stale() {
        // logged in at hour 0, asking at hour 0 plus a couple minutes
        assert!(!is_reauth_stale(at(0), at(0) + Duration::minutes(2)));
    }

    #[test]
    fn exactly_at_freshness_boundary_is_not_stale() {
        assert!(!is_reauth_stale(at(0), at(0) + Duration::minutes(5)));
    }

    #[test]
    fn past_freshness_window_is_stale() {
        assert!(is_reauth_stale(
            at(0),
            at(0) + Duration::minutes(5) + Duration::seconds(1)
        ));
    }

    #[test]
    fn hours_old_session_is_stale() {
        assert!(is_reauth_stale(at(0), at(6)));
    }

    #[test]
    fn in_grace_is_not_expired() {
        // deprecated at hour 0, now at hour 24, 48h grace → still valid
        assert!(!is_grace_expired(at(0), at(24), GRACE_48H));
    }

    #[test]
    fn exactly_at_deadline_is_not_expired() {
        // at the exact boundary we're still inside; strictly > means at == not expired
        assert!(!is_grace_expired(at(0), at(48), GRACE_48H));
    }

    #[test]
    fn past_deadline_is_expired() {
        // 1 second past the 48h grace
        let deadline = at(0) + Duration::hours(48);
        assert!(is_grace_expired(
            at(0),
            deadline + Duration::seconds(1),
            GRACE_48H
        ));
    }

    #[test]
    fn zero_grace_means_immediate_expiry_next_moment() {
        assert!(!is_grace_expired(at(0), at(0), 0));
        assert!(is_grace_expired(at(0), at(0) + Duration::seconds(1), 0));
    }

    #[test]
    fn long_grace_keeps_key_valid() {
        // 30-day grace
        let grace = 30 * 24 * 3600;
        assert!(!is_grace_expired(at(0), at(24 * 20), grace));
        assert!(is_grace_expired(at(0), at(24 * 31), grace));
    }
}
