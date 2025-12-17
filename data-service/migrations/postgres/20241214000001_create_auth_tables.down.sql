-- Rollback auth tables migration

-- Drop cleanup function
DROP FUNCTION IF EXISTS cleanup_expired_challenges(INTERVAL);

-- Drop store tables (reverse order of creation)
DROP TABLE IF EXISTS user_stores;
DROP TABLE IF EXISTS store_roles;
DROP TABLE IF EXISTS stores;

-- Drop challenge tables
DROP TABLE IF EXISTS wallet_challenges;
DROP TABLE IF EXISTS passkey_authentication_challenges;
DROP TABLE IF EXISTS passkey_registration_challenges;

-- Drop credential tables
DROP TABLE IF EXISTS wallet_credentials;
DROP TABLE IF EXISTS passkey_credentials;

-- Drop session and device tables
DROP TABLE IF EXISTS sessions;
DROP TABLE IF EXISTS devices;

-- Drop users table
DROP TABLE IF EXISTS users;

-- Drop device type enum
DROP TYPE IF EXISTS device_type;
