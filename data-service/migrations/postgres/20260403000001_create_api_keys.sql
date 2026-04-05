-- API Keys for programmatic access to the ethpayserver API.
-- Keys are scoped to a user and can be revoked without affecting sessions.

CREATE TABLE api_keys (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    -- Store only the hash of the key; the plaintext is shown once at creation.
    key_hash TEXT NOT NULL UNIQUE,
    -- Prefix for identification (e.g., "ak_live_****1234").
    key_prefix TEXT NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ
);

CREATE INDEX idx_api_keys_user_id ON api_keys(user_id);
CREATE INDEX idx_api_keys_key_hash ON api_keys(key_hash);
