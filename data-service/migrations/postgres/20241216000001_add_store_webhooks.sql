-- Store webhooks configuration
-- Allows merchants to receive notifications on invoice status changes

CREATE TABLE store_webhooks (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    store_id UUID NOT NULL REFERENCES stores(id) ON DELETE CASCADE,

    -- Webhook configuration
    webhook_url TEXT NOT NULL,
    webhook_secret TEXT NOT NULL,  -- HMAC secret for signing payloads
    enabled BOOLEAN NOT NULL DEFAULT true,

    -- Optional filters (NULL = all events)
    -- Can be extended later for event type filtering

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- One webhook config per store (can be extended to multiple later)
    CONSTRAINT unique_store_webhook UNIQUE (store_id)
);

-- Index for lookups by store
CREATE INDEX idx_store_webhooks_store_id ON store_webhooks(store_id);
CREATE INDEX idx_store_webhooks_enabled ON store_webhooks(store_id, enabled) WHERE enabled = true;

-- Trigger to update updated_at
CREATE TRIGGER store_webhooks_updated_at
    BEFORE UPDATE ON store_webhooks
    FOR EACH ROW
    EXECUTE FUNCTION update_tokens_updated_at();

COMMENT ON TABLE store_webhooks IS 'Webhook configuration for store invoice notifications';
COMMENT ON COLUMN store_webhooks.webhook_secret IS 'HMAC-SHA256 secret for signing webhook payloads';
