use uuid::Uuid;

/// Load test configuration, read from environment variables.
pub struct Config {
    /// API key for authentication (format: ak_XXXX_YYYY).
    pub api_key: String,
    /// Store ID to use for invoice operations.
    pub store_id: Uuid,
    /// Webhook receiver URL for the webhook-burst scenario.
    /// Defaults to http://localhost:9999/webhook if unset.
    pub webhook_receiver_url: String,
}

impl Config {
    pub fn from_env() -> Self {
        let api_key = std::env::var("LOADTEST_API_KEY").expect("LOADTEST_API_KEY is required");
        let store_id = std::env::var("LOADTEST_STORE_ID")
            .expect("LOADTEST_STORE_ID is required")
            .parse::<Uuid>()
            .expect("LOADTEST_STORE_ID must be a valid UUID");
        let webhook_receiver_url = std::env::var("LOADTEST_WEBHOOK_RECEIVER_URL")
            .unwrap_or_else(|_| "http://localhost:9999/webhook".to_string());

        Self {
            api_key,
            store_id,
            webhook_receiver_url,
        }
    }
}
