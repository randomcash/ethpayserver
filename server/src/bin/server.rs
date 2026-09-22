//! ETHPayServer main binary.
//!
//! Start the server with:
//! ```bash
//! DATABASE_URL="postgres://..." cargo run --release
//! ```

use secrecy::ExposeSecret;
use std::borrow::Cow;
use std::sync::Arc;

use anyhow::Result;
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use auth::{AuthConfig, AuthService, captcha::CloudflareTurnstile};
use data_service::PgDataService;
use evm::monitor::bridge::{COMMANDS_CHANNEL, EVENTS_CHANNEL, RedisBridge};
use rates::RateProviderConfig;
use server::{
    AppState, ChainHealthMetricsConfig, ChainHealthMetricsService, CleanupConfig, EventConsumer,
    InvoiceCleanupService, RedisEVMMonitor, WatchRetryConfig, WatchRetryService, WebhookConfig,
    WebhookService, api,
    api::api_key_rate_limit::ApiKeyRateLimitState,
    api::rate_limit::{RateLimitConfig, RateLimitState},
    config::Config,
    metrics,
};
use server::{
    DEFAULT_CALL_DEADLINE, DEFAULT_MAX_FAILURES, DEFAULT_MAX_IN_FLIGHT, PluginArtifacts,
    PluginHost, PluginPools, host_version, invoice_creation_filters, load_installed_plugins,
    own_store_payment_reporting, payment_observers, report_boot,
};

#[tokio::main]
#[allow(clippy::too_many_lines)] // server bootstrap — config, DB, services, routes in sequence
async fn main() -> Result<()> {
    // Load .env file if present
    let _ = dotenvy::dotenv();

    // Initialize Sentry (no-op when SENTRY_DSN is unset)
    let (_sentry_guard, sentry_dsn_configured, sentry_environment) =
        evm::telemetry::init_sentry(option_env!("CI_COMMIT_SHORT_SHA").map(Cow::from));

    // Load configuration
    let config = Config::from_env()?;

    // Initialize tracing (includes Sentry layer when DSN is configured)
    init_tracing(&config.log_level, &config.log_format);

    // Report whether error reporting is actually on. `tracing::info!` before
    // this point has no subscriber to write to, so this must come after
    // `init_tracing`, not next to `init_sentry`.
    evm::telemetry::report_reporting_status(sentry_dsn_configured, &sentry_environment)?;

    // Initialize Prometheus metrics
    metrics::init_metrics()?;

    tracing::info!("Starting ETHPayServer v{}", env!("CARGO_PKG_VERSION"));

    if config.safe_mode {
        tracing::warn!(
            "SAFE MODE: ETHPAY_DISABLE_PLUGINS is set - every plugin (including billing) is \
             disabled for this boot. Plugins are not uninstalled and their data is untouched; \
             clear the flag and restart to bring them back."
        );
    }

    // Connect to database
    tracing::info!("Connecting to database...");
    let data_service = Arc::new(PgDataService::connect(config.database_url.expose_secret()).await?);
    tracing::info!("Database connected");

    // Create auth service with custom config
    // Note: PgDataService implements AuthRepository (all required traits)
    let mut auth_config = AuthConfig {
        // Increase wallet challenge timeout to 10 minutes for registration flow
        wallet_challenge_duration: chrono::Duration::minutes(10),
        ..AuthConfig::default()
    };

    // WebAuthn config from environment
    let rp_id_explicit = std::env::var("WEBAUTHN_RP_ID").ok();
    if let Ok(rp_origin) = std::env::var("WEBAUTHN_RP_ORIGIN") {
        auth_config.rp_origin = rp_origin;
    }
    if let Ok(rp_name) = std::env::var("WEBAUTHN_RP_NAME") {
        auth_config.rp_name = rp_name;
    }

    // Set rp_id: use explicit env var, or auto-derive from rp_origin's host.
    if let Some(rp_id) = rp_id_explicit {
        auth_config.rp_id = rp_id;
    } else if let Some(host) = server::config::derive_rp_id_from_origin(&auth_config.rp_origin) {
        auth_config.rp_id = host;
    }

    // Validate WebAuthn config at startup to fail fast on misconfiguration
    if !server::config::validate_rp_id(&auth_config.rp_id, &auth_config.rp_origin) {
        tracing::error!(
            rp_id = %auth_config.rp_id,
            rp_origin = %auth_config.rp_origin,
            "WEBAUTHN_RP_ID must be a registrable domain suffix of WEBAUTHN_RP_ORIGIN host — passkey auth will fail"
        );
    }

    tracing::info!(rp_id = %auth_config.rp_id, rp_origin = %auth_config.rp_origin, "WebAuthn configured");

    // Captured here, from the resolved config, because `with_config` below moves
    // it into AuthService and AuthService keeps it private. /health/deep reports
    // these so the deploy check can stop scraping the log line just above - which
    // writes `rp_id` and `=` in separate ANSI escape sequences, so a literal
    // `rp_id=` matches nothing and the first version of that check failed against
    // a perfectly healthy server.
    let webauthn_health = api_types::WebAuthnHealth {
        rp_id: auth_config.rp_id.clone(),
        rp_origin: auth_config.rp_origin.clone(),
    };

    let auth_service = Arc::new(AuthService::with_config(
        Arc::clone(&data_service),
        auth_config,
    ));

    // Configure CAPTCHA provider (optional)
    let captcha_provider = match server::config::parse_captcha_env()? {
        Some((_provider, secret, site_key)) => {
            tracing::info!(provider = "turnstile", "CAPTCHA enabled");
            Some(Arc::new(CloudflareTurnstile::new(secret, site_key))
                as Arc<dyn auth::captcha::CaptchaProvider>)
        }
        None => {
            tracing::info!("CAPTCHA disabled (no CAPTCHA_PROVIDER set)");
            None
        }
    };

    // Connect to Redis (REQUIRED for event processing)
    // One explicit expose for the three consumers below (bridge, webhooks,
    // idempotency); the URL stays a secret everywhere else.
    let redis_url = config
        .redis_url
        .as_ref()
        .map(ExposeSecret::expose_secret)
        .ok_or_else(|| anyhow::anyhow!("REDIS_URL is required for event processing"))?;

    tracing::info!("Connecting to Redis...");
    let events_channel =
        std::env::var("REDIS_EVENTS_CHANNEL").unwrap_or_else(|_| EVENTS_CHANNEL.to_string());
    let commands_channel =
        std::env::var("REDIS_COMMANDS_CHANNEL").unwrap_or_else(|_| COMMANDS_CHANNEL.to_string());
    tracing::debug!(
        events_channel,
        commands_channel,
        "Redis channels configured"
    );
    let bridge = Arc::new(RedisBridge::new(redis_url, &events_channel, &commands_channel).await?);
    tracing::info!("Redis connected");

    // Create EVM monitor using shared bridge (concrete type for generics)
    let evm_monitor = Arc::new(RedisEVMMonitor::new(Arc::clone(&bridge)));

    // Create WebSocket broadcast channel (shared by services and HTTP handler)
    let ws_broadcast = Arc::new(server::api::ws::WsBroadcast::new(256));

    // Start background services
    // Bring up the plugin host and load whatever is installed.
    //
    // In safe mode there is no host at all - not an empty one. A boot that
    // builds no wasmtime engine cannot run plugin code by any path, including
    // one added later by someone who did not know to check the flag.
    let plugin_host = if config.safe_mode {
        None
    } else {
        Some(Arc::new(PluginHost::new(
            host_version(),
            DEFAULT_MAX_FAILURES,
            DEFAULT_CALL_DEADLINE,
        )))
    };
    let plugin_artifacts = PluginArtifacts::new(&config.plugin_dir);
    // A failure to *read* the install list is different from a plugin failing
    // to load: the database is not answering, which the rest of the boot is
    // about to discover anyway. Log and continue with no plugins rather than
    // refuse to start - a server that will not come up is the one state an
    // admin cannot fix a plugin problem from.
    // One pool per plugin, all sharing one budget. Built before the loader so
    // a plugin with a credential gets database access as it registers, rather
    // than on a later pass that would leave the first call after boot without
    // it.
    let plugin_pools = Arc::new(PluginPools::new(
        config.database_url.expose_secret().to_string(),
        DEFAULT_MAX_IN_FLIGHT,
    ));

    // The billing store: the stored setting wins, the environment is the
    // fallback.
    //
    // That precedence and not the reverse. An admin who sets this in the UI
    // must see it take effect - if an environment variable silently overrode
    // it, the settings page would show one store while the server billed on
    // another, and nothing would say so. The environment stays supported
    // because instances configured before this setting existed are still
    // configured that way, and because a fresh database has no settings row
    // to read.
    let billing_store_id =
        match auth::ServerSettingsRepository::get_server_settings(&*data_service).await {
            Ok(settings) => settings.and_then(|s| s.billing_store_id).or_else(|| {
                if config.billing_store_id.is_some() {
                    tracing::info!(
                        "billing store taken from ETHPAY_BILLING_STORE_ID; setting it in \
                     the admin settings takes precedence from then on"
                    );
                }
                config.billing_store_id
            }),
            // Not fatal. Falling back to the environment is the behaviour this
            // server had before the setting existed, and refusing to start over
            // an unreadable settings row would take the whole instance down for a
            // feature most instances do not use.
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "could not read server settings; falling back to ETHPAY_BILLING_STORE_ID"
                );
                config.billing_store_id
            }
        };

    // Published further down, once there is an `AppState` to build an issuer
    // around. Handed to the loader now because this is where a plugin is
    // given its host calls, and a plugin that got none here would have no
    // way to be granted them later.
    let plugin_capabilities = server::services::plugins::DeferredCapabilities::default();
    let plugin_issuer = plugin_capabilities.issuer.clone();

    let loaded = match load_installed_plugins(
        &*data_service,
        plugin_host.as_deref(),
        &plugin_artifacts,
        Some(&plugin_pools),
        &plugin_capabilities,
    )
    .await
    {
        Ok(report) => {
            report_boot(&report);
            report.loaded
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                "could not read the installed-plugin list; starting with no plugins loaded"
            );
            Vec::new()
        }
    };

    // Turn the loaded plugins into the capability implementations the rest of
    // the server calls. Without this, a plugin compiles, instantiates and
    // registers - and nothing ever dispatches to it.
    let (plugin_filters, plugin_payment_observers): (
        Vec<Arc<dyn server::services::plugins::InvoiceCreationFilter>>,
        Vec<Arc<dyn server::services::plugins::OwnStorePaymentObserver>>,
    ) = match plugin_host.as_ref() {
        Some(host) => (
            invoice_creation_filters(host, &loaded),
            payment_observers(host, &loaded),
        ),
        None => (Vec::new(), Vec::new()),
    };

    // Capability 4 needs both a store to watch and something to tell. Either
    // one missing means no dispatch at all: an instance with a billing store
    // but no plugin has nobody to notify, and observers without a configured
    // store must never be handed a guess at which store is ours.
    let own_store_payments =
        own_store_payment_reporting(billing_store_id, plugin_payment_observers);
    match own_store_payments.as_ref() {
        Some((store_id, observers)) => tracing::info!(
            %store_id,
            observers = observers.len() as u64,
            "own-store payments will be reported to plugins"
        ),
        None => tracing::info!(
            "no own-store payment reporting: ETHPAY_BILLING_STORE_ID unset or no plugin loaded"
        ),
    }

    // 1. Webhook delivery service - sends webhook notifications
    //    Created first because cleanup service needs it for expiration webhooks
    let webhook_config = WebhookConfig::from_env();
    tracing::debug!(?webhook_config, "Webhook config loaded");
    let webhook_service = Arc::new(WebhookService::new(
        Arc::clone(&data_service),
        redis_url,
        webhook_config,
    )?);
    tokio::spawn(Arc::clone(&webhook_service).run());
    tracing::info!("Webhook delivery service started");

    // 2. Invoice cleanup service - expires invoices and unwatches addresses
    //    Also queues webhook notifications when invoices expire
    let cleanup_config = CleanupConfig::from_env();
    tracing::debug!(?cleanup_config, "Cleanup config loaded");
    let cleanup_service = Arc::new(InvoiceCleanupService::new(
        Arc::clone(&data_service),
        Arc::clone(&evm_monitor),
        cleanup_config,
        Some(Arc::clone(&webhook_service)),
        Some(Arc::clone(&ws_broadcast)),
    ));
    tokio::spawn(Arc::clone(&cleanup_service).run());
    tracing::info!("Invoice cleanup service started");

    // 3. Event consumer - processes PaymentDetected/Confirmed events
    //    Also triggers expiration checks on block events and queues webhooks
    let bridge_dyn: Arc<dyn evm::monitor::bridge::EventBridge> = bridge.clone();
    let email_sender = server::services::create_email_sender();
    let event_consumer = EventConsumer::new(
        bridge_dyn,
        Arc::clone(&data_service),
        Some(cleanup_service),
        Some(Arc::clone(&webhook_service) as Arc<dyn server::services::WebhookSink>),
        Some(Arc::clone(&ws_broadcast)),
        Arc::clone(&email_sender),
    );
    // Cloned rather than moved: the same observers are also handed to the
    // reconciliation loop below, which is the pull path under this push one.
    let event_consumer = match own_store_payments.clone() {
        Some((store_id, observers)) => event_consumer.with_own_store_payments(store_id, observers),
        None => event_consumer,
    };
    tokio::spawn(event_consumer.run());
    tracing::info!("Event consumer started");

    // 4. Watch retry service - retries failed WatchAddress commands
    let retry_config = WatchRetryConfig::from_env();
    tracing::debug!(?retry_config, "Watch retry config loaded");
    if retry_config.enabled {
        let retry_service = WatchRetryService::new(
            Arc::clone(&data_service),
            Arc::clone(&evm_monitor),
            retry_config,
        );
        tokio::spawn(retry_service.run());
        tracing::info!("Watch retry service started");
    } else {
        tracing::info!("Watch retry service disabled");
    }

    // 5. Chain health metrics service - polls evmmonitor's published health
    //    data on a timer and exports it as Prometheus gauges, so the gauges
    //    are a fact about the chain rather than a side effect of someone
    //    calling /health/chains.
    let chain_health_metrics_config = ChainHealthMetricsConfig::from_env();
    tracing::debug!(
        ?chain_health_metrics_config,
        "Chain health metrics config loaded"
    );
    let chain_health_metrics_service =
        ChainHealthMetricsService::new(Arc::clone(&evm_monitor), chain_health_metrics_config);
    tokio::spawn(chain_health_metrics_service.run());
    tracing::info!("Chain health metrics service started");

    // Create rate provider
    let rate_config = RateProviderConfig::from_env();
    tracing::info!(provider = rate_config.provider, "Rate provider configured");
    let rate_provider = rate_config.create_provider();

    // Create application state
    let mut state = AppState::new(
        Arc::clone(&data_service),
        auth_service,
        Some(evm_monitor),
        rate_provider,
        email_sender,
    );
    state.ws_broadcast = Some(ws_broadcast);
    state.webhook_sink = Some(webhook_service);
    state.captcha_provider = captcha_provider;
    state.webauthn = Some(webauthn_health);
    state.safe_mode = config.safe_mode;

    state.plugin_host = plugin_host.clone();
    state.plugin_dir = config.plugin_dir.clone();
    // So install and uninstall reach the same pools the boot loader registered.
    state.plugin_pools = Some(Arc::clone(&plugin_pools));
    // The one wired filter call site. Empty until a filter plugin is
    // installed, which is every deployment today; before this line it was
    // empty even then.
    state.invoice_creation_filters = plugin_filters;
    // Never filtered: see `AppState::billing_store_id`.
    state.billing_store_id = billing_store_id;

    // Capability 3, published. An instance with no configured billing store
    // publishes nothing, and its plugins are told invoicing is unavailable -
    // which is the truth: there is no store this host would issue on, and
    // guessing at one is how a plugin ends up invoicing a merchant's
    // customers.
    match billing_store_id {
        Some(store_id) => {
            let api = Arc::new(server::services::plugins::PluginHostApi::new(
                state.clone(),
                store_id,
            ));
            let issuer: Arc<dyn server::services::plugins::HostInvoiceIssuer> = api.clone();
            if plugin_issuer.publish(issuer) {
                tracing::info!(%store_id, "plugins may issue invoices on this instance's own store");
            }

            // The pull half of own-store payment reporting. Push is the fast
            // path and never the source of truth: a dispatch is lost whenever
            // the plugin could not take it - disabled after repeated failure,
            // trapped, past its deadline, or not loaded because this process
            // was restarting when the payment confirmed. Every one of those
            // is a merchant who paid and was not credited, and none of them
            // is visible, because the payment itself succeeded.
            if let Some((_, observers)) = own_store_payments.as_ref() {
                let reader: Arc<dyn server::services::plugins::OwnStorePaymentReader> = api;
                tokio::spawn(server::services::plugins::reconcile::run(
                    reader,
                    observers.clone(),
                    server::services::plugins::reconcile::DEFAULT_INTERVAL,
                ));
                tracing::info!(
                    interval_secs =
                        server::services::plugins::reconcile::DEFAULT_INTERVAL.as_secs(),
                    "reconciling own-store payments on a loop; a lost dispatch is caught here"
                );
            }
        }
        None => tracing::info!(
            "plugins cannot issue invoices: ETHPAY_BILLING_STORE_ID is unset, so this \
             instance has no store of its own to bill on"
        ),
    }

    // Capability 6, published unconditionally. Unlike capability 3 it needs
    // no own store: an instance that sells nothing still has merchants with
    // volume, and a plugin asking what one settled deserves the real answer
    // rather than silence that reads as zero.
    if plugin_capabilities.volume.publish(Arc::new(
        server::services::plugins::PluginMerchantVolume::new(state.clone()),
    )) {
        tracing::info!("plugins may read what an account settled over a window");
    }

    // Capability 5. `PageHost` is built empty by `AppState::new` and has
    // never had a production renderer registered in it, so every plugin page
    // request 404'd - correct for a host with nothing to draw, and
    // indistinguishable from a feature that was never wired up.
    if let Some(host) = plugin_host.as_ref() {
        let mut pages = server::services::plugins::PageHost::new();
        for id in &loaded {
            pages.register(
                id.clone(),
                Arc::new(server::services::plugins::WasmPageRenderer::new(
                    Arc::clone(host),
                    id.clone(),
                )),
            );
        }
        if !loaded.is_empty() {
            tracing::info!(
                plugins = loaded.len() as u64,
                "plugin pages are served from /plugins/{{id}}/pages/{{path}}"
            );
        }
        state.plugin_pages = Arc::new(pages);
    }

    // Create rate limiters
    let rate_limit_config = RateLimitConfig::from_env();
    tracing::info!(?rate_limit_config, "Rate limiting configured");
    let rate_limiters = Arc::new(RateLimitState::from_config(&rate_limit_config));

    // Create idempotency state (shares the existing Redis connection)
    let idempotency = match api::idempotency::IdempotencyState::from_env(redis_url).await {
        Ok(idem) => {
            tracing::info!(ttl_secs = idem.ttl_secs, "Idempotency middleware enabled");
            Some(Arc::new(idem))
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to create idempotency state, disabled");
            None
        }
    };

    // Create per-API-key rate limiter
    let api_key_rate_limiter =
        Arc::new(ApiKeyRateLimitState::from_env(data_service.pool().clone()));
    tracing::info!(
        default_rpm = api_key_rate_limiter.default_rpm,
        "Per-API-key rate limiting configured"
    );

    // Build router with middleware
    let app = api::router(
        state,
        config.enable_swagger,
        Some(rate_limiters),
        idempotency,
        Some(api_key_rate_limiter),
    )
    .layer(TraceLayer::new_for_http())
    .layer(
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any),
    )
    // Performance-tracing transaction per request, named from the matched
    // route (`tower-axum-matched-path`) rather than the raw request URI —
    // otherwise every distinct invoice/store/etc. id mints its own
    // transaction name. Axum runs middleware in the reverse order it's
    // `.layer()`-ed, so `NewSentryLayer` must be added last to end up
    // outermost of `SentryHttpLayer`, per sentry-tower's documented ordering.
    .layer(sentry::integrations::tower::SentryHttpLayer::new().enable_transaction())
    .layer(sentry::integrations::tower::NewSentryLayer::<axum::extract::Request>::new_from_top());

    // Start server
    let bind_addr = config.bind_address();
    tracing::info!("Server listening on http://{}", bind_addr);

    if config.enable_swagger {
        tracing::info!("Swagger UI available at http://{}/swagger-ui", bind_addr);
    }

    let listener = TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Whether `log_format` selects JSON output, and a warning to log for a value
/// that is neither `json` nor `pretty` — an unrecognized value (a typo, wrong
/// case) would otherwise silently fall back to the human-readable format a
/// log shipper can't parse, with no signal that anything is wrong.
fn resolve_log_format(log_format: &str) -> (bool, Option<String>) {
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

fn init_tracing(log_level: &str, log_format: &str) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level));

    // `json` is what a log shipper (Grafana Cloud's Loki agent) parses; any
    // other value keeps the human-readable format for local/dev use.
    let (json, warning) = resolve_log_format(log_format);

    if json {
        tracing_subscriber::registry()
            .with(filter)
            .with(sentry_tracing::layer())
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(sentry_tracing::layer())
            .with(tracing_subscriber::fmt::layer())
            .init();
    }

    // Logged after `.init()` on purpose: there is no subscriber to write to
    // before that.
    if let Some(warning) = warning {
        tracing::warn!("{warning}");
    }
}

#[cfg(test)]
mod tracing_config_tests {
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
