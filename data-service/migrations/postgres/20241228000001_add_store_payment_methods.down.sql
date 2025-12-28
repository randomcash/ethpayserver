-- Revert store payment methods migration

ALTER TABLE watched_addresses DROP COLUMN IF EXISTS chain_id;
ALTER TABLE invoices DROP COLUMN IF EXISTS chain_id;
ALTER TABLE invoices DROP COLUMN IF EXISTS currency;
DROP TABLE IF EXISTS store_payment_methods;
