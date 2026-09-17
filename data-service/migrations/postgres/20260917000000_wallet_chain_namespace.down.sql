-- Reverting this loses which family each key belongs to, and that information
-- exists nowhere else - it cannot be read back out of an xpub. Refuse while
-- any wallet outside `eip155` exists rather than drop the column and leave a
-- Tron key indistinguishable from an Ethereum one in a schema that will then
-- derive Ethereum addresses from it.
DO $$
DECLARE other INTEGER;
BEGIN
    SELECT COUNT(*) INTO other FROM wallets WHERE namespace <> 'eip155';
    IF other > 0 THEN
        RAISE EXCEPTION
            '% wallet(s) are registered for a chain family other than eip155. '
            'Dropping the namespace column would make them indistinguishable '
            'from Ethereum keys and every address derived from them '
            'unreachable by their owner. Move or delete those wallets first.',
            other;
    END IF;
END $$;

ALTER TABLE store_wallets DROP CONSTRAINT store_wallets_wallet_family_fkey;
ALTER TABLE store_wallets
    ADD CONSTRAINT store_wallets_wallet_id_fkey
        FOREIGN KEY (wallet_id) REFERENCES wallets(id) ON DELETE RESTRICT;
ALTER TABLE store_wallets DROP CONSTRAINT store_wallets_pkey;
-- The pre-namespace schema can express one override per store. The guard above
-- has already established that every wallet is `eip155`, and an override's
-- namespace equals its wallet's by foreign key, so no store can hold two - and
-- re-adding the primary key will say so loudly if that reasoning is ever
-- wrong. Deleting the surplus rows to make it fit would be this migration
-- quietly choosing where a store's money goes.
ALTER TABLE store_wallets ADD PRIMARY KEY (store_id);
ALTER TABLE store_wallets DROP COLUMN namespace;

ALTER TABLE store_payment_methods DROP CONSTRAINT store_payment_methods_wallet_family_fkey;
ALTER TABLE store_payment_methods
    ADD CONSTRAINT store_payment_methods_wallet_id_fkey
        FOREIGN KEY (wallet_id) REFERENCES wallets(id) ON DELETE RESTRICT;
ALTER TABLE store_payment_methods DROP COLUMN chain_namespace;

DROP INDEX idx_account_wallets_id_namespace;

DROP INDEX idx_account_wallets_user_xpub;
CREATE UNIQUE INDEX idx_account_wallets_user_xpub ON wallets(user_id, xpub);

DROP INDEX idx_account_wallets_one_primary;
CREATE UNIQUE INDEX idx_account_wallets_one_primary
    ON wallets(user_id) WHERE is_primary;

ALTER TABLE wallets DROP COLUMN namespace;

DROP DOMAIN caip2_namespace;
