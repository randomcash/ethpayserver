-- An account's standing, as decided elsewhere and pushed here.
--
-- Keyed by account and deliberately not a foreign key to `users`: a push may
-- arrive before the account exists here, and a constraint would turn that into
-- an error the sender retries forever. Nothing reads a row except by a real
-- account id, so an early row is inert.
--
-- `version` only ever rises (the apply is a compare-and-set, not an update).
-- `received_at` is when this version was applied; `last_heard_at` is the last
-- time the sender confirmed it, which is what a freshness bound reads.
CREATE TABLE IF NOT EXISTS account_standing (
    account_id        UUID PRIMARY KEY,
    version           BIGINT NOT NULL CHECK (version > 0),
    in_good_standing  BOOLEAN NOT NULL,
    paid_through      TIMESTAMPTZ NULL,
    plan_name         TEXT NOT NULL CHECK (plan_name <> '' AND octet_length(plan_name) <= 200),
    checkout_url      TEXT NULL CHECK (checkout_url IS NULL OR
                          (octet_length(checkout_url) <= 2048 AND checkout_url LIKE 'https://%')),
    received_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_heard_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
