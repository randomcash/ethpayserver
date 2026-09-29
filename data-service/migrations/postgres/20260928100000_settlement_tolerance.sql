-- Per-store settlement tolerance: how far below the invoice amount a payment
-- may fall and still settle it, as a percentage of the invoice amount.
--
-- Kept beside `stores` rather than on it, like `store_token_policies`. A store
-- with no row uses the server default. The ceiling is enforced at the API,
-- which refuses rather than clamps; the CHECK is the backstop.
CREATE TABLE IF NOT EXISTS store_settlement_settings (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    store_id           UUID NOT NULL UNIQUE REFERENCES stores(id) ON DELETE CASCADE,
    -- Ceiling matches MAX_TOLERANCE_PERCENT in `settlement_tolerance.rs`. A
    -- backstop looser than the rule it backs up is not a backstop: a row
    -- between the two would be legal here and refused by the API, and since
    -- settlement fails closed on an unreadable tolerance, that store's
    -- invoices would stop settling rather than settle too easily.
    tolerance_percent  NUMERIC(78,18) NOT NULL CHECK (tolerance_percent >= 0 AND tolerance_percent <= 0.1),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Audit: an invoice that settled because of a tolerance, rather than by being
-- paid in full, records the shortfall accepted and the setting that allowed it.
CREATE TABLE IF NOT EXISTS invoice_settlement_allowances (
    invoice_id         VARCHAR(64) PRIMARY KEY REFERENCES invoices(id) ON DELETE CASCADE,
    shortfall          NUMERIC(78,18) NOT NULL,
    tolerance_percent  NUMERIC(78,18) NOT NULL,
    source             VARCHAR(16) NOT NULL CHECK (source IN ('store', 'default')),
    recorded_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
