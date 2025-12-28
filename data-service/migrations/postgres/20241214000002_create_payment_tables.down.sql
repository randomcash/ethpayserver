-- Rollback payment tables migration

-- Drop triggers first
DROP TRIGGER IF EXISTS trigger_update_invoice_on_payment ON payments;
DROP TRIGGER IF EXISTS tokens_updated_at ON tokens;

-- Drop functions
DROP FUNCTION IF EXISTS update_invoice_on_payment();
DROP FUNCTION IF EXISTS expire_old_invoices();
DROP FUNCTION IF EXISTS update_tokens_updated_at();

-- Drop tables in dependency order
DROP TABLE IF EXISTS payment_events;
DROP TABLE IF EXISTS payments;
DROP TABLE IF EXISTS watched_addresses;
DROP TABLE IF EXISTS payment_options;
DROP TABLE IF EXISTS invoices;
DROP TABLE IF EXISTS chain_configs;
DROP TABLE IF EXISTS tokens;

-- Drop enums
DROP TYPE IF EXISTS invoice_status;
DROP TYPE IF EXISTS asset_type;
