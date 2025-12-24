-- Revert wallet configuration

ALTER TABLE invoices DROP COLUMN IF EXISTS derivation_index;
DROP TABLE IF EXISTS store_wallets;
