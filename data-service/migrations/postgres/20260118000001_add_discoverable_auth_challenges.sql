-- Add table for discoverable authentication challenges
-- Used for passkey login without requiring email (discoverable credentials)

CREATE TABLE IF NOT EXISTS discoverable_authentication_challenges (
    challenge_id UUID PRIMARY KEY,
    challenge_state JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for cleanup of expired challenges
CREATE INDEX idx_discoverable_auth_challenges_created_at
    ON discoverable_authentication_challenges(created_at);
