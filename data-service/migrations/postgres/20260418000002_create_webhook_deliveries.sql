-- Webhook delivery tracking table.
-- Records every delivery attempt for audit and retry management.
--
-- Originally shipped as 20260418000001 alongside api_key_rate_limit, which
-- had the same version prefix and caused a _sqlx_migrations PK collision on
-- fresh DBs. Renamed to …000002; every statement below is now idempotent so
-- re-running on environments where it already ran under the old version is
-- a no-op rather than an error.

CREATE TABLE IF NOT EXISTS webhook_deliveries (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    store_webhook_id UUID NOT NULL,
    invoice_id      TEXT NOT NULL,
    event_type      TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending',
    attempts        INT  NOT NULL DEFAULT 0,
    max_attempts    INT  NOT NULL DEFAULT 7,
    last_error      TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Query by webhook
CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_store_webhook_id
    ON webhook_deliveries (store_webhook_id);

-- Query by invoice
CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_invoice_id
    ON webhook_deliveries (invoice_id);

-- Find deliveries needing retry
CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_status
    ON webhook_deliveries (status)
    WHERE status IN ('pending', 'retrying');

-- Time-based queries / cleanup
CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_created_at
    ON webhook_deliveries (created_at);

-- Trigger to auto-update updated_at (reuses the function from
-- earlier migrations if it exists).
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ language 'plpgsql';

-- Drop-then-create so re-runs replace any existing trigger rather than
-- erroring on "trigger already exists".
DROP TRIGGER IF EXISTS set_updated_at ON webhook_deliveries;
CREATE TRIGGER set_updated_at
    BEFORE UPDATE ON webhook_deliveries
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();
