-- RCS-241, down. Reversible only while every chain is still an EVM chain.
--
-- A BIGINT column can hold an EIP-155 number and nothing else. `eip155:1` goes
-- back to `1` losslessly; `tron:728126428` and
-- `solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp` have no BIGINT representation at
-- all. Tron's reference is numeric, which makes it the dangerous case: it would
-- cast cleanly to 728126428 and then be indistinguishable from an EIP-155 id,
-- so a rollback would silently hand a Tron chain to EVM code.
--
-- So: reverse where the reversal is exact, and refuse where it is not. Rolling
-- back a bad deploy on an EVM-only database is what a down migration is
-- actually for, and that case is lossless.

DO $$
DECLARE foreign_chains TEXT;
BEGIN
    -- Every column at once, so the error names all of them rather than failing
    -- on the first and hiding the rest.
    SELECT string_agg(DISTINCT chain, ', ') INTO foreign_chains
    FROM (
        SELECT chain_id::text AS chain FROM chain_configs
        UNION SELECT chain_id::text FROM invoices WHERE chain_id IS NOT NULL
        UNION SELECT chain_id::text FROM payment_options
        UNION SELECT chain_id::text FROM payments
        UNION SELECT chain_id::text FROM payouts
        UNION SELECT chain_id::text FROM refunds
        UNION SELECT chain_id::text FROM store_payment_methods
        UNION SELECT chain_id::text FROM store_token_policy_entries
        UNION SELECT chain_id::text FROM tokens
        UNION SELECT chain_id::text FROM watched_addresses
        UNION SELECT default_chain_id::text FROM store_settings WHERE default_chain_id IS NOT NULL
        UNION SELECT unnest(enabled_chain_ids)::text FROM server_settings
    ) AS every_chain
    WHERE chain NOT LIKE 'eip155:%';

    IF foreign_chains IS NOT NULL THEN
        RAISE EXCEPTION
            'RCS-241 down: these chains have no EIP-155 representation and '
            'cannot go back into a BIGINT column: %. Note that a numeric '
            'reference does not help - tron:728126428 would cast to a number '
            'and then be read as an EIP-155 id by EVM code. Restore from a '
            'pre-migration backup instead.', foreign_chains;
    END IF;
END $$;

-- Payment method ids first, while the chain is still readable in them.
UPDATE payment_options
SET payment_method_id =
        split_part(payment_method_id, '@', 1)
        || '-'
        || replace(split_part(payment_method_id, '@', 2), 'eip155:', '')
WHERE payment_method_id LIKE '%@eip155:%';

ALTER TABLE server_settings
    ALTER COLUMN enabled_chain_ids TYPE BIGINT[]
    USING (
        string_to_array(
            replace(array_to_string(enabled_chain_ids, ',', ''), 'eip155:', ''),
            ','
        )::BIGINT[]
    );

ALTER TABLE store_settings
    ALTER COLUMN default_chain_id TYPE BIGINT
    USING (CASE WHEN default_chain_id IS NULL
                THEN NULL
                ELSE replace(default_chain_id::text, 'eip155:', '')::BIGINT END);

ALTER TABLE watched_addresses
    ALTER COLUMN chain_id TYPE BIGINT USING (replace(chain_id::text, 'eip155:', '')::BIGINT);
ALTER TABLE tokens
    ALTER COLUMN chain_id TYPE BIGINT USING (replace(chain_id::text, 'eip155:', '')::BIGINT);
ALTER TABLE store_token_policy_entries
    ALTER COLUMN chain_id TYPE BIGINT USING (replace(chain_id::text, 'eip155:', '')::BIGINT);
ALTER TABLE store_payment_methods
    ALTER COLUMN chain_id TYPE BIGINT USING (replace(chain_id::text, 'eip155:', '')::BIGINT);
ALTER TABLE refunds
    ALTER COLUMN chain_id TYPE BIGINT USING (replace(chain_id::text, 'eip155:', '')::BIGINT);
ALTER TABLE payouts
    ALTER COLUMN chain_id TYPE BIGINT USING (replace(chain_id::text, 'eip155:', '')::BIGINT);
ALTER TABLE payments
    ALTER COLUMN chain_id TYPE BIGINT USING (replace(chain_id::text, 'eip155:', '')::BIGINT);
ALTER TABLE payment_options
    ALTER COLUMN chain_id TYPE BIGINT USING (replace(chain_id::text, 'eip155:', '')::BIGINT);
ALTER TABLE invoices
    ALTER COLUMN chain_id TYPE BIGINT USING (replace(chain_id::text, 'eip155:', '')::BIGINT);
ALTER TABLE chain_configs
    ALTER COLUMN chain_id TYPE BIGINT USING (replace(chain_id::text, 'eip155:', '')::BIGINT);

DROP DOMAIN IF EXISTS caip2;
