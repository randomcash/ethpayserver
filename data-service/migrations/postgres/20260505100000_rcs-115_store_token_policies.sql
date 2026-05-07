-- Store token policies: per-store allowlist/blocklist for accepted chain+token pairs.
-- RCS-115

CREATE TABLE IF NOT EXISTS store_token_policies (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    store_id    UUID NOT NULL UNIQUE REFERENCES stores(id) ON DELETE CASCADE,
    mode        VARCHAR(16) NOT NULL CHECK (mode IN ('allowlist', 'blocklist')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS store_token_policy_entries (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    policy_id       UUID NOT NULL REFERENCES store_token_policies(id) ON DELETE CASCADE,
    chain_id        BIGINT NOT NULL,
    token_address   VARCHAR(42),
    asset_symbol    VARCHAR(32) NOT NULL,
    UNIQUE (policy_id, chain_id, token_address)
);

CREATE INDEX IF NOT EXISTS idx_store_token_policy_entries_policy
    ON store_token_policy_entries(policy_id);

-- Auto-update updated_at on policy changes.
CREATE OR REPLACE FUNCTION update_store_token_policy_timestamp()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_store_token_policies_updated ON store_token_policies;
CREATE TRIGGER trg_store_token_policies_updated
    BEFORE UPDATE ON store_token_policies
    FOR EACH ROW
    EXECUTE FUNCTION update_store_token_policy_timestamp();
