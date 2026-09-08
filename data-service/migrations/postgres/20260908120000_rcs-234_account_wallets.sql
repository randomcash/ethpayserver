-- RCS-234: move the xpub and its derivation counter up to the account.
--
-- Until now every `store_payment_methods` row carried its own `xpub` AND its
-- own `derivation_index`. Address derivation is `m/44'/60'/0'/0/{index}`
-- (evm/src/wallet.rs) - no store, chain or asset appears in the path - so two
-- rows holding the same xpub with independent counters derive byte-identical
-- addresses. Two merchants' payments then arrive at one address, attribution
-- is guesswork, and the funds are mixed.
--
-- That is not hypothetical and it is not only a cross-store problem. The
-- ordinary way to configure a store is to paste one xpub for ETH and the same
-- xpub for USDC; those are two rows, two counters, and their indices 0, 1, 2
-- are the same three addresses. On testnet one xpub is currently spread over
-- 16 payment-method rows all sitting at index 1: sixteen rows, one address.
--
-- The fix is structural, not procedural. An xpub and the counter that consumes
-- it become one row in `wallets`, owned by the account. Payment methods and
-- stores reference a wallet. There is then exactly one counter per key, and no
-- amount of misconfiguration can produce a second one.
--
-- Deliberately one file, so it is one implicit transaction. Splitting it would
-- leave a window where `store_payment_methods.wallet_id` is NULL and the
-- server, mid-deploy, cannot derive an address at all. The tables touched here
-- are small (payment methods and payment options, not invoices), so holding
-- the lock for the rewrite is cheap - unlike RCS-201, which had to split for
-- exactly the opposite reason.

-- ---------------------------------------------------------------------------
-- 1. The account wallet
-- ---------------------------------------------------------------------------

CREATE TABLE wallets (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,

    xpub VARCHAR(120) NOT NULL,

    -- The single counter for this key. Nothing else in the schema may hold a
    -- derivation counter; that is the invariant.
    derivation_index INTEGER NOT NULL DEFAULT 0,

    name VARCHAR(100),
    is_primary BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index names are prefixed `idx_account_wallets_` rather than `idx_wallets_`:
-- the login-credential table `wallet_credentials` already owns `idx_wallets_*`
-- (20241214000001), and index names are per-schema, not per-table. There are
-- two unrelated things called a wallet in this database and the namespace
-- collides between them.
CREATE INDEX idx_account_wallets_user ON wallets(user_id);

-- Exactly one primary per account, enforced here rather than in application
-- code. Two concurrent "make this my primary" requests cannot both win, and no
-- future writer can forget the rule. Promotion must demote first, in the same
-- transaction - the index is immediate, not deferred.
CREATE UNIQUE INDEX idx_account_wallets_one_primary
    ON wallets(user_id) WHERE is_primary;

-- One row per (account, xpub). A second row for the same key would be a second
-- counter on it, which is the whole bug. Note the scope: this stops an account
-- colliding with itself. It cannot stop two different accounts registering the
-- same xpub, because the backfill below has no correct way to assign one
-- shared key to one owner, and picking a winner would hand one merchant's
-- wallet to another. That residual case is rejected at the API layer instead.
CREATE UNIQUE INDEX idx_account_wallets_user_xpub ON wallets(user_id, xpub);

COMMENT ON TABLE wallets IS
    'Account-level receiving wallets. One xpub, one derivation counter, per '
    'row - the pairing that makes duplicate address derivation impossible '
    '(RCS-234).';
COMMENT ON COLUMN wallets.derivation_index IS
    'Next derivation index to issue. Advanced only by an atomic UPDATE ... '
    'RETURNING; never read-then-written.';
COMMENT ON COLUMN wallets.is_primary IS
    'The wallet stores fall back to when they have no override. At most one '
    'per user (idx_wallets_one_primary_per_user).';

-- ---------------------------------------------------------------------------
-- 2. store_wallets: a reference, not a wallet
-- ---------------------------------------------------------------------------
--
-- The name is reused deliberately. The original table was created by
-- 20241215000001 and dropped again by 20241228000001 when payment methods took
-- over, so nothing at current head holds one; what comes back carries no xpub
-- and no counter, only a pointer. An absent row means "use the account
-- primary", which is why this is a table and not a nullable column on stores:
-- absent and NULL would be the same state, but a row lets the override be
-- deleted without touching the store.

CREATE TABLE store_wallets (
    store_id UUID PRIMARY KEY REFERENCES stores(id) ON DELETE CASCADE,
    -- RESTRICT, not CASCADE: deleting a wallet that a store still points at
    -- would silently move that store onto the primary and change where its
    -- money goes. Make the caller unpick it.
    wallet_id UUID NOT NULL REFERENCES wallets(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_store_wallets_wallet_id ON store_wallets(wallet_id);

COMMENT ON TABLE store_wallets IS
    'Per-store wallet override. No row means the store uses the account '
    'primary (RCS-234).';

-- ---------------------------------------------------------------------------
-- 3. Point payment methods at wallets
-- ---------------------------------------------------------------------------

ALTER TABLE store_payment_methods
    ADD COLUMN wallet_id UUID REFERENCES wallets(id) ON DELETE RESTRICT;

-- One wallet per (owner, xpub), carrying the highest index any of the rows
-- that shared that key had reached.
--
-- MAX is the load-bearing word. Methods that shared an xpub each counted
-- independently, so the addresses already issued from that key are the union
-- of every row's range: a method at 5 has issued 0..4, one at 3 has issued
-- 0..2, and the union is 0..4. Starting the shared counter at MAX (5) is the
-- only choice that never re-issues an address. MIN or a per-row copy would
-- hand out an address that has already been given to a customer and may
-- already hold money. The cost is skipped indices, which is free - the
-- derivation path has 2^31 of them and gaps are unobservable.
--
-- What this cannot undo: rows that already shared a key have already issued
-- the same low addresses more than once. Those collisions are in the data
-- before this migration runs and are a deploy-time cleanup, not a schema fix.
INSERT INTO wallets (user_id, xpub, derivation_index)
SELECT s.owner_id, pm.xpub, MAX(pm.derivation_index)
FROM store_payment_methods pm
JOIN stores s ON s.id = pm.store_id
GROUP BY s.owner_id, pm.xpub;

UPDATE store_payment_methods pm
SET wallet_id = w.id
FROM stores s, wallets w
WHERE s.id = pm.store_id
  AND w.user_id = s.owner_id
  AND w.xpub = pm.xpub;

-- Every method must have landed on a wallet; a NULL here would mean a store
-- with no owner, which the FK on stores.owner_id already forbids. Fail loudly
-- rather than let the NOT NULL below report it as a generic constraint error.
DO $$
DECLARE orphaned INTEGER;
BEGIN
    SELECT COUNT(*) INTO orphaned FROM store_payment_methods WHERE wallet_id IS NULL;
    IF orphaned > 0 THEN
        RAISE EXCEPTION
            'RCS-234: % payment method(s) could not be matched to an account '
            'wallet. Refusing to continue rather than derive from an unknown '
            'key.', orphaned;
    END IF;
END $$;

ALTER TABLE store_payment_methods ALTER COLUMN wallet_id SET NOT NULL;

CREATE INDEX idx_store_payment_methods_wallet ON store_payment_methods(wallet_id);

-- The columns that made the collision possible. Readers get them back through
-- a join, so `StorePaymentMethod` keeps the same shape in Rust; what changes
-- is that there is now one place they can be written.
ALTER TABLE store_payment_methods DROP COLUMN xpub;
ALTER TABLE store_payment_methods DROP COLUMN derivation_index;

COMMENT ON COLUMN store_payment_methods.wallet_id IS
    'Account wallet this method derives from. The xpub and counter live there '
    '(RCS-234).';

-- ---------------------------------------------------------------------------
-- 4. Elect a primary per account
-- ---------------------------------------------------------------------------
--
-- The wallet backing the most payment methods, because that is the one the
-- account has in practice been using; ties broken by id so the choice is
-- deterministic and a re-run of the migration on a copy picks the same wallet.
UPDATE wallets SET is_primary = TRUE
WHERE id IN (
    SELECT DISTINCT ON (w.user_id) w.id
    FROM wallets w
    LEFT JOIN store_payment_methods pm ON pm.wallet_id = w.id
    GROUP BY w.user_id, w.id
    ORDER BY w.user_id, COUNT(pm.id) DESC, w.id
);

-- ---------------------------------------------------------------------------
-- 5. Preserve today's routing exactly
-- ---------------------------------------------------------------------------
--
-- Before this migration every store derived from the xpub on its own payment
-- methods, so it was already, in effect, overriding. Writing an explicit
-- override for each of those stores keeps behaviour identical on day one.
-- Leaving them to fall through to the primary would be a silent change of
-- payout destination for every store whose wallet is not the elected primary,
-- which is the one thing a migration must never do.
--
-- Stores with no payment methods get no row and follow the primary, which is
-- the new default for anything created from here on.
INSERT INTO store_wallets (store_id, wallet_id)
SELECT DISTINCT ON (pm.store_id) pm.store_id, pm.wallet_id
FROM store_payment_methods pm
GROUP BY pm.store_id, pm.wallet_id
ORDER BY pm.store_id, COUNT(*) DESC, pm.wallet_id;

-- ---------------------------------------------------------------------------
-- 6. Provenance on payment options
-- ---------------------------------------------------------------------------
--
-- RCS-234 asked for `invoices.wallet_id`. That is the wrong grain: an invoice
-- has one payment option per accepted asset, each with its own address, and
-- once stores can share wallets those options can even come from different
-- keys. The column belongs on the option.
--
-- Nothing is unresolvable without it - `payment_options.payment_address` has
-- always recorded the literal address, so no historical invoice is at risk.
-- What this adds is the ability to answer "which key produced this address,
-- and at what index" without re-deriving every candidate.
ALTER TABLE payment_options
    ADD COLUMN wallet_id UUID REFERENCES wallets(id),
    ADD COLUMN derivation_index INTEGER;

-- Backfill what is recoverable. The option records chain and token but not
-- which payment-method row served it, so it is matched back through the
-- invoice's store. `IS NOT DISTINCT FROM` because token_address is NULL for
-- native assets and `= NULL` matches nothing.
UPDATE payment_options po
SET wallet_id = pm.wallet_id
FROM invoices i
JOIN store_payment_methods pm ON pm.store_id = i.store_id
WHERE po.invoice_id = i.id
  AND pm.chain_id = po.chain_id
  AND pm.token_address IS NOT DISTINCT FROM po.token_address;

-- `derivation_index` is deliberately left NULL on historical rows: the index
-- an old option used was never recorded anywhere, and inventing one by
-- re-deriving would be a guess dressed up as data. NULL means "issued before
-- RCS-234", and readers must treat it as unknown rather than as zero.
CREATE INDEX idx_payment_options_wallet ON payment_options(wallet_id)
    WHERE wallet_id IS NOT NULL;

COMMENT ON COLUMN payment_options.wallet_id IS
    'Wallet whose xpub produced payment_address. NULL for options created '
    'before RCS-234 whose method could not be matched back.';
COMMENT ON COLUMN payment_options.derivation_index IS
    'Index used within wallet_id. NULL means pre-RCS-234 and unknown - not 0.';
