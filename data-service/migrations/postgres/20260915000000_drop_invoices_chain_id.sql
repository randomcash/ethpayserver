-- invoices.chain_id is dead. It was added alongside store_payment_methods for
-- a network enum that never shipped, and no code has read or written it since
-- - an invoice is deliberately network-agnostic; the chains it can be paid on
-- live on payment_options. Left in place, it invites the next reader to
-- assume an invoice has a chain and build on a column that has always been
-- NULL.
ALTER TABLE invoices DROP COLUMN chain_id;
