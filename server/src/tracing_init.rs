use tracing_subscriber::{Layer, layer::SubscriberExt, util::SubscriberInitExt};

/// Whether `log_format` selects JSON output, and a warning to log for a value
/// that is neither `json` nor `pretty` — an unrecognized value (a typo, wrong
/// case) would otherwise silently fall back to the human-readable format a
/// log shipper can't parse, with no signal that anything is wrong.
pub fn resolve_log_format(log_format: &str) -> (bool, Option<String>) {
    match log_format {
        "json" => (true, None),
        "pretty" => (false, None),
        other => (
            false,
            Some(format!(
                "LOG_FORMAT={other:?} is not \"json\" or \"pretty\"; defaulting to pretty"
            )),
        ),
    }
}

pub fn init_tracing(log_level: &str, log_format: &str) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level));

    // `json` is what a log shipper (Grafana Cloud's Loki agent) parses; any
    // other value keeps the human-readable format for local/dev use.
    let (json, warning) = resolve_log_format(log_format);

    // Gates which levels become Sentry *structured logs* specifically, so
    // testnet can ship INFO there while mainnet ships WARN and above. Applied
    // as the Sentry layer's own per-layer filter (below) rather than folded
    // into `filter`, because a bare `.with(filter)` layer sits in the same
    // `Layered` stack as every other layer and `Layered::enabled` ANDs across
    // all of them — an event `filter` (LOG_LEVEL) rejects never reaches the
    // Sentry layer's `on_event` at all, so `SENTRY_LOG_LEVEL` could only ever
    // be a *further* restriction on top of LOG_LEVEL, never independent of
    // it. Per-layer filtering (`.with_filter` on each layer instead of a
    // shared `.with(filter)`) is what actually decouples them.
    let sentry_log_level = evm::telemetry::resolve_sentry_log_level();
    // Floor for the Sentry layer's own callsite interest, independent of
    // LOG_LEVEL. Fixed at INFO because `sentry_tracing`'s event/span
    // classification never does anything below INFO regardless of
    // `sentry_log_level` (DEBUG/TRACE are always `EventFilter::Ignore`), so
    // this can't suppress anything `sentry_log_event_filter` would keep.
    let sentry_filter = tracing_subscriber::filter::LevelFilter::INFO;

    if json {
        tracing_subscriber::registry()
            .with(
                sentry_tracing::layer()
                    .event_filter(evm::telemetry::sentry_log_event_filter(sentry_log_level))
                    .with_filter(sentry_filter),
            )
            .with(tracing_subscriber::fmt::layer().json().with_filter(filter))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(
                sentry_tracing::layer()
                    .event_filter(evm::telemetry::sentry_log_event_filter(sentry_log_level))
                    .with_filter(sentry_filter),
            )
            .with(tracing_subscriber::fmt::layer().with_filter(filter))
            .init();
    }

    // Logged after `.init()` on purpose: there is no subscriber to write to
    // before that.
    if let Some(warning) = warning {
        tracing::warn!("{warning}");
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_log_format;

    #[test]
    fn json_selects_json_with_no_warning() {
        assert_eq!(resolve_log_format("json"), (true, None));
    }

    #[test]
    fn pretty_selects_pretty_with_no_warning() {
        assert_eq!(resolve_log_format("pretty"), (false, None));
    }

    #[test]
    fn unrecognized_value_falls_back_to_pretty_with_a_warning() {
        let (json, warning) = resolve_log_format("JSON");
        assert!(!json);
        assert!(
            warning.is_some(),
            "a typo'd LOG_FORMAT must not fail silently"
        );
    }
}
