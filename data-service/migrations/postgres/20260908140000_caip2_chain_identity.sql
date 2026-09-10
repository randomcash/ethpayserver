-- Chain identity becomes CAIP-2.
--
-- Every chain column here was `BIGINT` holding an EIP-155 chain id. That names
-- exactly one family of chains. Tron, Solana, Monero and Bitcoin have no
-- EIP-155 id, so the roadmap (EVM -> Tron -> Solana -> Monero -> Bitcoin) had
-- two options: invent numbers for them, or carry a second identifier alongside
-- the first. Both end with two ways to say which chain a payment arrived on.
--
-- CAIP-2 is `namespace:reference` - `eip155:1`, `tron:728126428`,
-- `solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp`. It is what the wallet ecosystem
-- already uses (CASA, with MetaMask, WalletConnect and Ledger participating),
-- so the identifier stored here is the one a wallet SDK expects. It is also
-- open: a namespace is a string, so a chain can be added without a schema
-- change, which is what the plugin architecture needs.
--
-- Every row in this database today is an EVM row, so `'eip155:' || chain_id` is
-- a complete and exact conversion. This is cheap now - one chain, 17 payment
-- methods - and would be extremely expensive once a second chain exists.
--
-- Column names do not change. CAIP-2 calls its identifier a "chain ID", so
-- `chain_id` stays the right name; what changes is the type. Any code that
-- missed the change fails loudly on the type, which is the point.

-- ---------------------------------------------------------------------------
-- 1. The shape of a CAIP-2 identifier
-- ---------------------------------------------------------------------------
--
-- Enforced in the database, not just in Rust. The Rust type validates on
-- construction, but this table is also written by migrations and by hand during
-- incidents, and a bare `1` slipping into a chain column is precisely the
-- pre-CAIP-2 state we are leaving. Kept as a reusable domain rather than
-- repeating the regex twelve times.
--
--   namespace: [-a-z0-9]{3,8}      lowercase only
--   reference: [-_a-zA-Z0-9]{1,32} case-sensitive (Solana's base58 needs it)
CREATE DOMAIN caip2 AS TEXT
    CHECK (VALUE ~ '^[-a-z0-9]{3,8}:[-_a-zA-Z0-9]{1,32}$');

COMMENT ON DOMAIN caip2 IS
    'A CAIP-2 chain identifier, e.g. eip155:1. Namespace is lowercase; the '
    'reference is case-sensitive and mostly opaque - only eip155 and tron '
    'references are numbers, the rest are truncated genesis hashes.';

-- ---------------------------------------------------------------------------
-- 2. Convert every chain column
-- ---------------------------------------------------------------------------
--
-- `USING` rewrites each row as it goes. Postgres rebuilds every dependent index
-- automatically - and there are sixteen of them, including the unique indexes
-- on (tx_hash, chain_id), (address, chain_id, token_address) and
-- (store_id, chain_id, token_address) - so they must not be dropped by hand
-- here. Uniqueness is preserved because the conversion is injective: distinct
-- BIGINTs give distinct 'eip155:N' strings.

ALTER TABLE chain_configs
    ALTER COLUMN chain_id TYPE caip2 USING ('eip155:' || chain_id::text);

ALTER TABLE invoices
    ALTER COLUMN chain_id TYPE caip2 USING ('eip155:' || chain_id::text);

ALTER TABLE payment_options
    ALTER COLUMN chain_id TYPE caip2 USING ('eip155:' || chain_id::text);

ALTER TABLE payments
    ALTER COLUMN chain_id TYPE caip2 USING ('eip155:' || chain_id::text);

ALTER TABLE payouts
    ALTER COLUMN chain_id TYPE caip2 USING ('eip155:' || chain_id::text);

ALTER TABLE refunds
    ALTER COLUMN chain_id TYPE caip2 USING ('eip155:' || chain_id::text);

ALTER TABLE store_payment_methods
    ALTER COLUMN chain_id TYPE caip2 USING ('eip155:' || chain_id::text);

ALTER TABLE store_token_policy_entries
    ALTER COLUMN chain_id TYPE caip2 USING ('eip155:' || chain_id::text);

ALTER TABLE tokens
    ALTER COLUMN chain_id TYPE caip2 USING ('eip155:' || chain_id::text);

ALTER TABLE watched_addresses
    ALTER COLUMN chain_id TYPE caip2 USING ('eip155:' || chain_id::text);

-- Nullable, so NULL has to survive rather than becoming the string 'eip155:'.
ALTER TABLE store_settings
    ALTER COLUMN default_chain_id TYPE caip2
    USING (CASE WHEN default_chain_id IS NULL
                THEN NULL
                ELSE 'eip155:' || default_chain_id::text END);

-- An array of them. `enabled_chain_ids` is how a server says which chains it
-- serves; as BIGINT[] a Tron server could not express its own chains at all.
--
-- Through a function because `ALTER ... USING` rejects a subquery, and mapping
-- over an array needs `unnest`. `WITH ORDINALITY` keeps the original order: the
-- first enabled chain is treated as the default in places, so re-ordering the
-- array would quietly change behaviour. Dropped again below - it exists only
-- for the length of this statement.
-- The column default is NOT converted by `USING` - that clause rewrites rows,
-- while the default is re-coerced by assignment cast and survives as an integer
-- array. It then violates the domain on any insert that omits the column:
--
--   INSERT INTO server_settings (id) VALUES (1);
--   ERROR: value for domain caip2 violates check constraint "caip2_check"
--
-- The application always binds the column, so this is latent there - but a
-- hand-written insert during an incident is exactly the case the domain exists
-- for, and it would fail confusingly. Dropped here, restored below.
ALTER TABLE server_settings ALTER COLUMN enabled_chain_ids DROP DEFAULT;

CREATE FUNCTION to_caip2(ids BIGINT[]) RETURNS caip2[] AS $fn$
    SELECT COALESCE(
        array_agg(('eip155:' || id::text)::caip2 ORDER BY ord),
        '{}'::caip2[]
    )
    FROM unnest(ids) WITH ORDINALITY AS t(id, ord);
$fn$ LANGUAGE sql IMMUTABLE;

ALTER TABLE server_settings
    ALTER COLUMN enabled_chain_ids TYPE caip2[]
    USING to_caip2(enabled_chain_ids);

DROP FUNCTION to_caip2(BIGINT[]);

ALTER TABLE server_settings
    ALTER COLUMN enabled_chain_ids SET DEFAULT ARRAY[
        'eip155:1', 'eip155:10', 'eip155:137', 'eip155:42161', 'eip155:8453',
        'eip155:56', 'eip155:43114', 'eip155:250', 'eip155:100', 'eip155:324',
        'eip155:59144', 'eip155:534352'
    ]::caip2[];

-- ---------------------------------------------------------------------------
-- 3. Payment method ids carry a chain too
-- ---------------------------------------------------------------------------
--
-- `payment_options.payment_method_id` is a string of the form `{ASSET}-{CHAIN}`
-- - 'ETH-11155111'. The separator changes to `@` along with the chain, because
-- a CAIP-2 reference may itself contain a hyphen (`cosmos:cosmoshub-3`) and the
-- parser split on the LAST one: 'ATOM-cosmos:cosmoshub-3' would have yielded a
-- chain called '3'. `@` appears in neither the CAIP-2 charset nor any asset
-- symbol.
--
-- Rewritten by splitting on the last hyphen, which is exactly what the old
-- parser did, so an asset symbol containing a hyphen ('WETH-USDC-1') converts
-- the same way it used to parse.
UPDATE payment_options
SET payment_method_id =
        left(payment_method_id, length(payment_method_id) - strpos(reverse(payment_method_id), '-'))
        || '@eip155:'
        || right(payment_method_id, strpos(reverse(payment_method_id), '-') - 1)
WHERE payment_method_id LIKE '%-%'
  AND payment_method_id NOT LIKE '%@%'
  -- Only rows whose trailing segment is a bare number: those are the old
  -- format. Anything else is already converted or was never in it.
  AND right(payment_method_id, strpos(reverse(payment_method_id), '-') - 1) ~ '^[0-9]+$';

-- Nothing may be left in the old shape. A surviving `ETH-1` would parse to no
-- chain at all once the code expects `@`, and the payment option would quietly
-- stop resolving.
DO $$
DECLARE stragglers INTEGER;
BEGIN
    SELECT COUNT(*) INTO stragglers
    FROM payment_options
    WHERE payment_method_id NOT LIKE '%@%';

    IF stragglers > 0 THEN
        RAISE EXCEPTION
            '% payment option(s) still carry a pre-CAIP-2 '
            'payment_method_id. They would resolve to no chain. Convert them '
            'before continuing.', stragglers;
    END IF;
END $$;

-- ---------------------------------------------------------------------------
-- 4. Say what the columns now mean
-- ---------------------------------------------------------------------------

COMMENT ON COLUMN chain_configs.chain_id IS
    'CAIP-2 identifier. This table is the mapping from identifier to display '
    'name, symbol, decimals and explorer - most references are opaque genesis '
    'hashes, so nothing may infer a name from an identifier.';
COMMENT ON COLUMN payments.chain_id IS 'CAIP-2 identifier of the chain this payment arrived on.';
COMMENT ON COLUMN payment_options.chain_id IS 'CAIP-2 identifier of the chain this option is payable on.';
COMMENT ON COLUMN watched_addresses.chain_id IS 'CAIP-2 identifier of the chain this address is watched on.';
COMMENT ON COLUMN server_settings.enabled_chain_ids IS
    'CAIP-2 identifiers this server serves. Was BIGINT[] of EIP-155 ids, which '
    'a non-EVM server could not populate.';
