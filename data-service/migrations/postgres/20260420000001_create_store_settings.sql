-- Store-level settings for defaults, branding, and notification preferences.
CREATE TABLE store_settings (
    store_id UUID PRIMARY KEY REFERENCES stores(id) ON DELETE CASCADE,
    default_chain_id BIGINT NULL,
    default_display_currency VARCHAR(8) NULL,
    logo_url TEXT NULL,
    accent_color CHAR(7) NULL,
    notification_prefs JSONB NOT NULL DEFAULT '{}',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
