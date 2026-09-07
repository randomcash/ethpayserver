//! Timestamp parsing and relative-time rendering for API rows.
//!
//! The client has no date library — `chrono` is not in the WASM dependency
//! set — so the two things the UI actually needs from an API timestamp are
//! implemented here instead of pulling one in. Both are pure functions taking
//! "now" as an argument so they can be tested without a clock.

/// Parse an RFC 3339 / ISO 8601 timestamp into milliseconds since the Unix
/// epoch.
///
/// Accepts what `serde` emits for `chrono::DateTime<Utc>`
/// (`2026-09-07T19:21:47.939123Z`) plus explicit numeric offsets
/// (`...+02:00`). Returns `None` for anything it cannot read rather than
/// guessing: a wrong timestamp renders as a wrong "2 minutes ago", which is
/// exactly the kind of invented detail RCS-224 is removing.
#[must_use]
pub fn parse_iso8601_ms(iso: &str) -> Option<i64> {
    let bytes = iso.as_bytes();
    // Shortest form we accept: YYYY-MM-DDTHH:MM:SS
    if bytes.len() < 19 {
        return None;
    }

    let num = |range: std::ops::Range<usize>| -> Option<i64> { iso.get(range)?.parse().ok() };

    let year = num(0..4)?;
    let month = num(5..7)?;
    let day = num(8..10)?;
    let hour = num(11..13)?;
    let minute = num(14..16)?;
    let second = num(17..19)?;

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let rest = &iso[19..];

    // Fractional seconds, if present, then the zone suffix.
    let (millis, zone) = if let Some(frac) = rest.strip_prefix('.') {
        let digits: String = frac.chars().take_while(char::is_ascii_digit).collect();
        let millis = digits
            .chars()
            .chain(std::iter::repeat('0'))
            .take(3)
            .collect::<String>()
            .parse::<i64>()
            .ok()?;
        (millis, &frac[digits.len()..])
    } else {
        (0, rest)
    };

    let offset_seconds = match zone.as_bytes().first() {
        None | Some(b'Z') | Some(b'z') => 0,
        Some(sign @ (b'+' | b'-')) => {
            if zone.len() < 6 {
                return None;
            }
            let oh: i64 = zone.get(1..3)?.parse().ok()?;
            let om: i64 = zone.get(4..6)?.parse().ok()?;
            let magnitude = oh * 3600 + om * 60;
            if *sign == b'-' { -magnitude } else { magnitude }
        }
        Some(_) => return None,
    };

    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second - offset_seconds;
    Some(seconds * 1000 + millis)
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
///
/// Howard Hinnant's `days_from_civil`, which is the standard branch-free way
/// to do this without a calendar library.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Render an API timestamp as a short relative time against `now_ms`.
///
/// Returns `None` when the timestamp is unparseable, so callers can show the
/// raw value instead of a fabricated one.
#[must_use]
pub fn relative_time(iso: &str, now_ms: f64) -> Option<String> {
    let then_ms = parse_iso8601_ms(iso)?;
    let elapsed_s = (now_ms as i64 - then_ms) / 1000;

    // Clock skew between the browser and the server routinely puts a
    // just-detected payment a few seconds in the future. "Just now" is true;
    // "in 4 seconds" reads as a bug.
    if elapsed_s < 60 {
        return Some("Just now".to_string());
    }

    let (value, unit) = if elapsed_s < 3_600 {
        (elapsed_s / 60, "min")
    } else if elapsed_s < 86_400 {
        (elapsed_s / 3_600, "hour")
    } else if elapsed_s < 2_592_000 {
        (elapsed_s / 86_400, "day")
    } else {
        (elapsed_s / 2_592_000, "month")
    };

    Some(if value == 1 {
        format!("1 {unit} ago")
    } else {
        format!("{value} {unit}s ago")
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_iso8601_ms, relative_time};

    /// 2026-09-07T19:21:47Z in epoch milliseconds.
    const REFERENCE_MS: i64 = 1_788_808_907_000;

    #[test]
    fn parses_the_epoch() {
        assert_eq!(parse_iso8601_ms("1970-01-01T00:00:00Z"), Some(0));
    }

    #[test]
    fn parses_what_chrono_serializes() {
        assert_eq!(parse_iso8601_ms("2026-09-07T19:21:47Z"), Some(REFERENCE_MS));
        // Sub-second precision: chrono emits microseconds, we keep millis.
        assert_eq!(
            parse_iso8601_ms("2026-09-07T19:21:47.939123Z"),
            Some(REFERENCE_MS + 939)
        );
        // A single fractional digit is tenths, not milliseconds.
        assert_eq!(
            parse_iso8601_ms("2026-09-07T19:21:47.5Z"),
            Some(REFERENCE_MS + 500)
        );
    }

    #[test]
    fn applies_numeric_offsets() {
        // Same instant, expressed in +02:00 and -05:00.
        assert_eq!(
            parse_iso8601_ms("2026-09-07T21:21:47+02:00"),
            Some(REFERENCE_MS)
        );
        assert_eq!(
            parse_iso8601_ms("2026-09-07T14:21:47-05:00"),
            Some(REFERENCE_MS)
        );
    }

    #[test]
    fn handles_leap_days() {
        assert_eq!(
            parse_iso8601_ms("2024-02-29T00:00:00Z"),
            Some(1_709_164_800_000)
        );
    }

    #[test]
    fn refuses_garbage_rather_than_guessing() {
        assert_eq!(parse_iso8601_ms(""), None);
        assert_eq!(parse_iso8601_ms("not a timestamp"), None);
        assert_eq!(parse_iso8601_ms("2026-09-07"), None);
        assert_eq!(parse_iso8601_ms("2026-13-07T00:00:00Z"), None);
        assert_eq!(parse_iso8601_ms("2026-09-07T25:00:00Z"), None);
        assert_eq!(parse_iso8601_ms("2026-09-07T19:21:47 CEST"), None);
    }

    #[test]
    fn buckets_elapsed_time() {
        let now = REFERENCE_MS as f64;
        let at = |secs: i64| {
            relative_time("2026-09-07T19:21:47Z", now + (secs * 1000) as f64).unwrap_or_default()
        };
        assert_eq!(at(0), "Just now");
        assert_eq!(at(59), "Just now");
        assert_eq!(at(60), "1 min ago");
        assert_eq!(at(125), "2 mins ago");
        assert_eq!(at(3_600), "1 hour ago");
        assert_eq!(at(7_200), "2 hours ago");
        assert_eq!(at(86_400), "1 day ago");
        assert_eq!(at(172_800), "2 days ago");
        assert_eq!(at(2_592_000), "1 month ago");
        assert_eq!(at(7_776_000), "3 months ago");
    }

    #[test]
    fn a_future_timestamp_is_just_now_not_negative() {
        // Browser clocks run ahead of the server's often enough that this is
        // the common case for a payment detected seconds ago.
        let now = REFERENCE_MS as f64;
        assert_eq!(
            relative_time("2026-09-07T19:22:47Z", now).as_deref(),
            Some("Just now")
        );
    }

    #[test]
    fn unparseable_input_has_no_relative_form() {
        assert_eq!(relative_time("whenever", 0.0), None);
    }
}
