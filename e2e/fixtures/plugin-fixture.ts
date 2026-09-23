/**
 * The e2e fixture plugin, read from the files that are the single source of
 * truth for it.
 *
 * The wasm, its WAT source and the manifest live in `fixtures/plugin/` as
 * plain files rather than as constants here, because two things need them:
 * this module, and `scripts/seed-plugin.mjs`, which runs as plain Node before
 * the server starts and cannot import TypeScript. Duplicating the manifest
 * across the two would be a fixture that disagrees with itself.
 *
 * See `fixtures/plugin/README.md` for what the module does and how to rebuild
 * it.
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const DIR = join(__dirname, 'plugin');

/** The module itself, base64'd for the install endpoint's JSON body. */
export const FIXTURE_PLUGIN_WASM_BASE64 = readFileSync(
  join(DIR, 'fixture.wasm'),
).toString('base64');

/** The manifest, verbatim — the endpoint parses the TOML it is given. */
export const FIXTURE_MANIFEST_TOML = readFileSync(
  join(DIR, 'plugin.toml'),
  'utf8',
);

/**
 * Read out of the manifest rather than repeated, so a fixture renamed in one
 * place cannot pass here while failing on the server.
 */
function field(name: string): string {
  const match = FIXTURE_MANIFEST_TOML.match(
    new RegExp(`^${name}\\s*=\\s*"([^"]+)"`, 'm'),
  );
  if (!match) throw new Error(`fixture manifest has no ${name}`);
  return match[1];
}

export const FIXTURE_PLUGIN_ID = field('id');
export const FIXTURE_PLUGIN_SLUG = field('slug');
export const FIXTURE_PLUGIN_VERSION = field('version');

/** What the fixture's `render_page` always answers with. */
export const FIXTURE_PAGE_TEXT = 'rendered by a plugin';
export const FIXTURE_PAGE_PATH = 'overview';
export const FIXTURE_ADMIN_PAGE_PATH = 'operators';

/**
 * A second plugin, for testing the install path itself.
 *
 * The seeded fixture is loaded at boot and proves rendering; this one is
 * installed while the server is running and proves what installing does —
 * including that it is *not* loaded, and so must not appear in navigation
 * until the server restarts.
 *
 * Same bytes, different identity: what is being tested is the endpoint and
 * the lifecycle, not the module.
 */
export const SECOND_PLUGIN_ID = 'com.example.e2elate';
export const SECOND_PLUGIN_SLUG = 'e2elate';
export const SECOND_MANIFEST_TOML = [
  `id = "${SECOND_PLUGIN_ID}"`,
  'version = "0.1.0"',
  'dependencies = ["ethpayserver:^0.1.0"]',
  'kind = "action"',
  `slug = "${SECOND_PLUGIN_SLUG}"`,
  '',
  '[[pages]]',
  'path = "overview"',
  'label = "Installed late"',
  '',
].join('\n');
