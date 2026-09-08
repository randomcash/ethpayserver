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

-- NULLABLE on purpose: NULL means "inherit", and resolution walks the store's
-- override and then the account primary. A method pinned to a wallet keeps
-- deriving from it whatever the account does; one left NULL follows the store.
-- Both states are reachable, so this is not a column waiting to be tightened.
ALTER TABLE store_payment_methods
    ADD COLUMN wallet_id UUID REFERENCES wallets(id) ON DELETE RESTRICT;

-- One wallet per (owner, xpub), each starting above every index that key has
-- issued ANYWHERE.
--
-- MAX is the load-bearing word, and its scope is the subtle part. Methods that
-- shared an xpub each counted independently, so the addresses already issued
-- from that key are the union of every row's range: a method at 5 has issued
-- 0..4, one at 3 has issued 0..2, and the union is 0..4. Starting the counter
-- at MAX (5) is the only choice that never re-issues an address; MIN or a
-- per-row copy hands out an address a customer already has.
--
-- The max is taken over the whole table, NOT per owner, even though the
-- wallets themselves are per owner. Ownership of a shared key cannot be
-- arbitrated - two accounts pasting one xpub is either co-custody or a
-- mistake, and picking a winner would hand one merchant's wallet to the other
-- - so each owner keeps a wallet row. But if each of those rows started at its
-- own owner's max, the lower one would then issue straight through the range
-- the other has already spent: owner A at 9 and owner B at 3 means B issues
-- 3..9, every one of which A has already given to a customer. Those would be
-- collisions the migration itself created, not ones it inherited. Every wallet
-- on a shared key therefore starts at the global high-water mark, and the
-- rows advance independently from there without ever colliding backwards.
--
-- The cost is skipped indices, which is free - the path has 2^31 of them and
-- gaps are unobservable.
--
-- What this cannot undo: rows that already shared a key have already issued
-- the same low addresses more than once. Those collisions are in the data
-- before this migration runs and are a deploy-time cleanup, not a schema fix.
INSERT INTO wallets (user_id, xpub, derivation_index)
SELECT s.owner_id, pm.xpub, high_water.max_index
FROM store_payment_methods pm
JOIN stores s ON s.id = pm.store_id
JOIN (
    SELECT xpub, MAX(derivation_index) AS max_index
    FROM store_payment_methods
    GROUP BY xpub
) AS high_water ON high_water.xpub = pm.xpub
GROUP BY s.owner_id, pm.xpub, high_water.max_index;

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

-- Deliberately NOT set to NOT NULL: see the column comment above. Every
-- existing method is pinned by the backfill, which is what keeps today's
-- routing identical; clearing a pin is how a store is later handed to its
-- override.
CREATE INDEX idx_store_payment_methods_wallet ON store_payment_methods(wallet_id)
    WHERE wallet_id IS NOT NULL;

-- The columns that made the collision possible. Readers get them back through
-- a join, so `StorePaymentMethod` keeps the same shape in Rust; what changes
-- is that there is now one place they can be written.
ALTER TABLE store_payment_methods DROP COLUMN xpub;
ALTER TABLE store_payment_methods DROP COLUMN derivation_index;

COMMENT ON COLUMN store_payment_methods.wallet_id IS
    'Wallet this method is pinned to. NULL means inherit: the store override '
    'if it has one, else the account primary. The xpub and counter live on '
    'the wallet (RCS-234).';

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
-- ON DELETE SET NULL, not the default RESTRICT. Provenance must never be the
-- reason a wallet cannot be removed: every wallet that has ever issued an
-- address would otherwise be pinned forever by its own history, and the API
-- would report "still in use" for a wallet nothing actually uses. The column
-- already means "unknown" when NULL, so losing it degrades exactly as an
-- unbackfillable historical row does.
ALTER TABLE payment_options
    ADD COLUMN wallet_id UUID REFERENCES wallets(id) ON DELETE SET NULL,
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
  AND pm.token_address IS NOT DISTINCT FROM po.token_address
  -- Skip methods that were rotated after this option was created. The method's
  -- current key is not the one that produced this address, and stamping it
  -- would assert a provenance that is simply wrong - worse than the NULL that
  -- honestly says "unknown". `wallet_rotations` records when each rotation
  -- happened, which is exactly enough to tell the two apart.
  AND NOT EXISTS (
      SELECT 1 FROM wallet_rotations wr
      WHERE wr.payment_method_id = pm.id
        AND wr.rotated_at > po.created_at
  );

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

-- ---------------------------------------------------------------------------
-- 7. Close the NULL gap in the payment-method uniqueness constraint
-- ---------------------------------------------------------------------------
--
-- `UNIQUE(store_id, chain_id, token_address)` does not constrain native assets
-- at all: token_address is NULL for them, and NULL is distinct from NULL in a
-- unique index. So every "add ETH on mainnet" inserted another row rather than
-- updating the existing one - the `ON CONFLICT` in `create_payment_method`
-- could never match - and one store accumulated several ETH methods, each
-- previously with its own xpub and counter.
--
-- That is the same duplicate-counter shape RCS-234 removes everywhere else,
-- reached through a constraint that silently does not apply. Left alone it
-- also makes "which wallet does this store use" ambiguous, since the answer is
-- picked from whichever duplicate sorts first.
--
-- Collapse duplicates deterministically before constraining: the oldest row
-- per (store, chain) survives, because it is the one whose addresses have been
-- in circulation longest and is most likely referenced by existing invoices.
-- Rotation history is repointed onto the survivor first - `wallet_rotations`
-- is ON DELETE CASCADE, so deleting a duplicate outright would silently
-- destroy the audit trail of a key that was rotated for a reason.
UPDATE wallet_rotations wr
SET payment_method_id = survivor.id
FROM (
    SELECT DISTINCT ON (store_id, chain_id) id, store_id, chain_id
    FROM store_payment_methods
    WHERE token_address IS NULL
    ORDER BY store_id, chain_id, created_at, id
) AS survivor
JOIN store_payment_methods dup
  ON dup.store_id = survivor.store_id
 AND dup.chain_id = survivor.chain_id
 AND dup.token_address IS NULL
 AND dup.id <> survivor.id
WHERE wr.payment_method_id = dup.id;

DELETE FROM store_payment_methods dup
USING (
    SELECT DISTINCT ON (store_id, chain_id) id, store_id, chain_id
    FROM store_payment_methods
    WHERE token_address IS NULL
    ORDER BY store_id, chain_id, created_at, id
) AS survivor
WHERE dup.token_address IS NULL
  AND dup.store_id = survivor.store_id
  AND dup.chain_id = survivor.chain_id
  AND dup.id <> survivor.id;

-- The constraint the table always meant to have. Partial, because it only
-- needs to cover the rows the composite unique index cannot see.
CREATE UNIQUE INDEX idx_store_payment_methods_native
    ON store_payment_methods(store_id, chain_id)
    WHERE token_address IS NULL;
