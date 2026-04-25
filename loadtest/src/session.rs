use uuid::Uuid;

/// Per-user session data shared across all Goose transactions.
#[derive(Debug, Clone)]
pub struct Session {
    pub api_key: String,
    pub store_id: Uuid,
    pub webhook_receiver_url: String,
}
