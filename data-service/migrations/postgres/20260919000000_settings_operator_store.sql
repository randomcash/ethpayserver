-- The store this instance bills its own subscriptions through.
--
-- Was `ETHPAY_BILLING_STORE_ID` only, which meant changing it needed a shell
-- on the deploy box and a file only the deploy user can write. It is a
-- product decision rather than infrastructure, so it belongs with the other
-- admin-configurable settings.
--
-- No foreign key to `stores`, deliberately. A settings row must not be what
-- stops a store being deleted, and a dangling id here is recoverable - the
-- instance reports no own-store payments and says so at boot - whereas a
-- foreign key would turn it into a delete that fails for a reason nobody
-- looking at the store would guess. The admin endpoint checks the store
-- exists at write time, which is where there is a human to tell.
ALTER TABLE server_settings
    ADD COLUMN IF NOT EXISTS billing_store_id UUID;

COMMENT ON COLUMN server_settings.billing_store_id IS
    'Store this instance issues its own subscription invoices on. Read at boot; '
    'a change takes effect on restart. NULL means the instance sells nothing to itself.';
