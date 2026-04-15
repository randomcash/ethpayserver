-- Webhook delivery log: records every delivery attempt (real and test).
CREATE TABLE webhook_deliveries (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    store_id UUID NOT NULL REFERENCES stores(id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    http_status SMALLINT,
    response_body TEXT,
    latency_ms INTEGER NOT NULL DEFAULT 0,
    success BOOLEAN NOT NULL DEFAULT false,
    error_message TEXT,
    attempt_number INTEGER NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_webhook_deliveries_store_id ON webhook_deliveries(store_id, created_at DESC);
CREATE INDEX idx_webhook_deliveries_success ON webhook_deliveries(store_id, success) WHERE NOT success;
