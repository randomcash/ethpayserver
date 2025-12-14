-- Rollback payment tables migration

-- Drop triggers first
DROP TRIGGER IF EXISTS trigger_update_invoice_on_payment ON payments;

-- Drop functions
DROP FUNCTION IF EXISTS update_invoice_on_payment();
DROP FUNCTION IF EXISTS expire_old_invoices();
DROP FUNCTION IF EXISTS get_asset_display(network, asset_type, UUID);

-- Drop tables in dependency order
DROP TABLE IF EXISTS payment_events;
DROP TABLE IF EXISTS watched_addresses;
DROP TABLE IF EXISTS payments;
DROP TABLE IF EXISTS invoices;
DROP TABLE IF EXISTS tokens;
DROP TABLE IF EXISTS network_configs;

-- Drop enums
DROP TYPE IF EXISTS invoice_status;
DROP TYPE IF EXISTS asset_type;
DROP TYPE IF EXISTS network;
