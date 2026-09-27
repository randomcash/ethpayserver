-- Whether this key's holder is exempt from invoice-creation entitlement
-- filters, independent of role or owned store.
--
-- Defaults false and is set nowhere by any endpoint in this server - granting
-- it is a deliberate, out-of-band act, not something a self-service create or
-- rotate call can produce. A `ServerAdmin` role does not imply this: an admin
-- session never carries it, so widening admin access never widens this.
ALTER TABLE api_keys ADD COLUMN is_operator BOOLEAN NOT NULL DEFAULT FALSE;

COMMENT ON COLUMN api_keys.is_operator IS
    'Exempts requests made with this key from invoice-creation entitlement filters. '
    'Explicitly granted only; never implied by role or ownership.';
