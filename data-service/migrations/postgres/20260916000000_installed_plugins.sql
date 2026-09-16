-- What is installed, so that it survives a restart.
--
-- The plugin host has, until now, been entirely in-memory: `PluginHost` and
-- `PluginRegistry` are built, registered into, and dropped with the process.
-- Nothing recorded that an admin had installed anything, so "installed"
-- could not outlive a boot and the host was reachable from no running
-- server at all.
--
-- One row per installed plugin, keyed by the plugin id, which is already
-- constrained to a safe charset by `PluginId::new`.
CREATE TABLE installed_plugins (
    id TEXT PRIMARY KEY,

    -- The plugin build's own semver, as its manifest declares it. Also names
    -- the artifact on disk, so an upgrade can keep the previous version's
    -- file rather than overwriting the only copy of a working plugin.
    version TEXT NOT NULL,

    -- The manifest exactly as it was accepted, verbatim.
    --
    -- `payserver_plugin_api::Manifest` is `Deserialize` only - it is parsed
    -- from TOML and never written back out - so there is no serialized form
    -- to store instead. Keeping the original text means boot re-runs the
    -- same parse and the same registry gate the install ran, rather than
    -- trusting a second representation that could drift from it.
    manifest_toml TEXT NOT NULL,

    -- SHA-256 of the wasm bytes at install time.
    --
    -- Checked again before the module is compiled on every boot. The
    -- artifact lives in a directory an admin can write to, and a plugin is
    -- code this server executes: a file that no longer hashes to what was
    -- installed is refused rather than run.
    artifact_sha256 TEXT NOT NULL,

    -- Whether this plugin loads on the next boot.
    --
    -- Deliberately persisted rather than recomputed. BTCPay's crash handling
    -- disables a plugin and restarts, and their users still report the UI
    -- coming back unreachable - a disable that lives only in memory is
    -- undone by the very restart it triggers, which is a crash loop rather
    -- than a recovery. A plugin the host turned off stays off until an admin
    -- turns it back on.
    enabled BOOLEAN NOT NULL DEFAULT TRUE,

    -- Why the host disabled it, in words an admin can act on. NULL whenever
    -- `enabled`, and whenever a human did the disabling.
    disabled_reason TEXT,

    installed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- What changed, when, and who did it.
--
-- Deliberately has no foreign key to `installed_plugins`: the most
-- interesting thing this table can tell an admin is that a plugin was
-- uninstalled, and a cascading delete would erase exactly that. The rows
-- outlive the plugin they describe.
CREATE TABLE plugin_events (
    id BIGSERIAL PRIMARY KEY,
    plugin_id TEXT NOT NULL,

    -- installed | upgraded | enabled | disabled | uninstalled | load_failed
    event TEXT NOT NULL,

    -- The version involved, where one applies.
    version TEXT,

    -- Free text: the trap message, the digest mismatch, the admin's reason.
    detail TEXT,

    -- The admin who did it, or NULL when the host did it to itself - a
    -- crash-disable has no actor, and saying so is more honest than
    -- attributing it to whoever happened to be logged in.
    actor_user_id UUID,

    at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_plugin_events_plugin_at ON plugin_events (plugin_id, at DESC);
