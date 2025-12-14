-- Auth tables migration
-- Creates tables for user authentication and device management

-- Enable UUID extension if not already enabled
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- =============================================================================
-- Device type enum
-- =============================================================================
CREATE TYPE device_type AS ENUM ('browser', 'desktop', 'mobile', 'api_client', 'unknown');

-- =============================================================================
-- Users table
-- =============================================================================
CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),

    -- Identifiers (at least one must be set)
    email VARCHAR(255) UNIQUE,
    primary_wallet_address VARCHAR(42) UNIQUE,

    -- Key derivation parameters (JSONB for flexibility)
    kdf_params JSONB NOT NULL,

    -- Encrypted symmetric key (client-encrypted with mnemonic-derived key)
    encrypted_symmetric_key JSONB NOT NULL,

    -- Recovery verification hash (for mnemonic verification during recovery)
    recovery_verification_hash VARCHAR(255) NOT NULL,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_login_at TIMESTAMPTZ,

    -- Rate limiting / lockout
    failed_login_attempts INTEGER NOT NULL DEFAULT 0,
    locked_until TIMESTAMPTZ,

    -- Constraints
    CONSTRAINT users_has_identifier CHECK (
        email IS NOT NULL OR primary_wallet_address IS NOT NULL
    )
);

-- Indexes for users
CREATE INDEX idx_users_email ON users(email) WHERE email IS NOT NULL;
CREATE INDEX idx_users_wallet ON users(primary_wallet_address) WHERE primary_wallet_address IS NOT NULL;
CREATE INDEX idx_users_locked ON users(locked_until) WHERE locked_until IS NOT NULL;

-- =============================================================================
-- Devices table
-- =============================================================================
CREATE TABLE devices (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,

    -- Device info
    name VARCHAR(255) NOT NULL,
    device_type device_type NOT NULL DEFAULT 'unknown',

    -- Device-specific encrypted key (same as user's, for device access)
    encrypted_symmetric_key JSONB NOT NULL,
    kdf_params JSONB NOT NULL,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at TIMESTAMPTZ,

    -- Status
    is_active BOOLEAN NOT NULL DEFAULT TRUE
);

-- Indexes for devices
CREATE INDEX idx_devices_user_id ON devices(user_id);
CREATE INDEX idx_devices_user_active ON devices(user_id, is_active) WHERE is_active = TRUE;

-- =============================================================================
-- Sessions table
-- =============================================================================
CREATE TABLE sessions (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    device_id UUID NOT NULL REFERENCES devices(id) ON DELETE CASCADE,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    last_activity_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Audit info
    ip_address INET,
    user_agent TEXT
);

-- Indexes for sessions
CREATE INDEX idx_sessions_user_id ON sessions(user_id);
CREATE INDEX idx_sessions_device_id ON sessions(device_id);
CREATE INDEX idx_sessions_expires_at ON sessions(expires_at);
CREATE INDEX idx_sessions_last_activity ON sessions(last_activity_at);

-- =============================================================================
-- Passkey credentials table (WebAuthn)
-- =============================================================================
CREATE TABLE passkey_credentials (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,

    -- Passkey metadata
    name VARCHAR(255) NOT NULL,

    -- WebAuthn passkey data (serialized webauthn-rs Passkey type)
    passkey JSONB NOT NULL,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at TIMESTAMPTZ,

    -- Status
    is_active BOOLEAN NOT NULL DEFAULT TRUE
);

-- Indexes for passkey_credentials
CREATE INDEX idx_passkeys_user_id ON passkey_credentials(user_id);
CREATE INDEX idx_passkeys_user_active ON passkey_credentials(user_id, is_active) WHERE is_active = TRUE;

-- =============================================================================
-- Wallet credentials table (Ethereum)
-- =============================================================================
CREATE TABLE wallet_credentials (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,

    -- Wallet info
    address VARCHAR(42) NOT NULL,  -- Checksummed EIP-55 address
    name VARCHAR(255) NOT NULL,
    is_primary BOOLEAN NOT NULL DEFAULT FALSE,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at TIMESTAMPTZ,

    -- Status
    is_active BOOLEAN NOT NULL DEFAULT TRUE,

    -- Ensure unique active wallet address
    CONSTRAINT unique_active_wallet_address UNIQUE (address)
);

-- Indexes for wallet_credentials
CREATE INDEX idx_wallets_user_id ON wallet_credentials(user_id);
CREATE INDEX idx_wallets_address ON wallet_credentials(address);
CREATE INDEX idx_wallets_user_active ON wallet_credentials(user_id, is_active) WHERE is_active = TRUE;
CREATE INDEX idx_wallets_user_primary ON wallet_credentials(user_id, is_primary) WHERE is_primary = TRUE;

-- =============================================================================
-- Challenge tables (ephemeral, for auth flows)
-- These store temporary challenge state during registration/login flows
-- =============================================================================

-- Passkey registration challenges
CREATE TABLE passkey_registration_challenges (
    user_id UUID PRIMARY KEY,
    identifier VARCHAR(255) NOT NULL,  -- email or wallet address for verification
    challenge_state JSONB NOT NULL,    -- serialized PasskeyRegistration
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for cleanup
CREATE INDEX idx_passkey_reg_challenges_created ON passkey_registration_challenges(created_at);

-- Passkey authentication challenges
CREATE TABLE passkey_authentication_challenges (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    challenge_state JSONB NOT NULL,    -- serialized PasskeyAuthentication
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for cleanup
CREATE INDEX idx_passkey_auth_challenges_created ON passkey_authentication_challenges(created_at);

-- Wallet challenges
CREATE TABLE wallet_challenges (
    user_id UUID PRIMARY KEY,  -- Can be temporary user_id for new registrations
    challenge VARCHAR(128) NOT NULL,   -- Random hex challenge
    address VARCHAR(42) NOT NULL,      -- Checksummed address
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for cleanup
CREATE INDEX idx_wallet_challenges_created ON wallet_challenges(created_at);

-- =============================================================================
-- Cleanup function for expired challenges
-- =============================================================================
CREATE OR REPLACE FUNCTION cleanup_expired_challenges(max_age INTERVAL DEFAULT '5 minutes')
RETURNS TABLE (
    passkey_reg_deleted BIGINT,
    passkey_auth_deleted BIGINT,
    wallet_deleted BIGINT
) AS $$
DECLARE
    cutoff TIMESTAMPTZ := NOW() - max_age;
BEGIN
    DELETE FROM passkey_registration_challenges WHERE created_at < cutoff;
    GET DIAGNOSTICS passkey_reg_deleted = ROW_COUNT;

    DELETE FROM passkey_authentication_challenges WHERE created_at < cutoff;
    GET DIAGNOSTICS passkey_auth_deleted = ROW_COUNT;

    DELETE FROM wallet_challenges WHERE created_at < cutoff;
    GET DIAGNOSTICS wallet_deleted = ROW_COUNT;

    RETURN NEXT;
END;
$$ LANGUAGE plpgsql;

-- =============================================================================
-- Comments for documentation
-- =============================================================================
COMMENT ON TABLE users IS 'User accounts with client-side encrypted key material';
COMMENT ON TABLE devices IS 'Registered devices that can access user accounts';
COMMENT ON TABLE sessions IS 'Active login sessions';
COMMENT ON TABLE passkey_credentials IS 'WebAuthn passkey credentials for passwordless auth';
COMMENT ON TABLE wallet_credentials IS 'Ethereum wallet credentials for Web3 auth';
COMMENT ON TABLE passkey_registration_challenges IS 'Ephemeral challenges for passkey registration (auto-cleanup)';
COMMENT ON TABLE passkey_authentication_challenges IS 'Ephemeral challenges for passkey login (auto-cleanup)';
COMMENT ON TABLE wallet_challenges IS 'Ephemeral challenges for wallet auth (auto-cleanup)';

COMMENT ON COLUMN users.kdf_params IS 'JSON: {algorithm, memory_mib, iterations, parallelism, salt_base64}';
COMMENT ON COLUMN users.encrypted_symmetric_key IS 'JSON: {ciphertext_base64, nonce_base64, tag_base64}';
COMMENT ON COLUMN users.recovery_verification_hash IS 'base64(SHA-256(Argon2id(mnemonic, identifier)))';
