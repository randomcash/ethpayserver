#!/usr/bin/env node
/**
 * Put a plugin on disk and in the database before the server starts.
 *
 * The host instantiates plugins at boot, so installing one through the API
 * leaves it recorded and not running - `POST /admin/plugins` says as much,
 * answering `restart_required: true`. A suite that installed its fixture and
 * then asked for a page would be testing a plugin that is not loaded, and the
 * honest result is a skip. A suite whose meaningful tests all skip is green
 * having tested nothing, which is the failure this repository keeps finding.
 *
 * So the fixture is seeded the way a restart would leave it: the artifact
 * written where `PluginArtifacts` looks (`<root>/<id>/<version>.wasm`), the
 * digest computed from those exact bytes, and the row written as the install
 * endpoint would write it. The server then loads it on its first boot and
 * every page test runs for real.
 *
 * The install *path* is still covered - `plugins.spec.ts` installs a second
 * plugin through the API and asserts what that does, including that an
 * unloaded plugin is correctly absent from navigation.
 *
 * Usage: node scripts/seed-plugin.mjs <plugin-dir>
 */
import { createHash } from 'node:crypto';
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname } from 'node:path';

import pg from 'pg';

const HERE = dirname(fileURLToPath(import.meta.url));
const PLUGIN_DIR_SRC = join(HERE, '..', 'fixtures', 'plugin');

// Read from the same files `fixtures/plugin-fixture.ts` reads, rather than
// repeating them: a manifest that disagreed between the seed and the spec
// would fail as "the plugin is not loaded", which says nothing about why.
const manifestToml = readFileSync(join(PLUGIN_DIR_SRC, 'plugin.toml'), 'utf8');
const wasm = readFileSync(join(PLUGIN_DIR_SRC, 'fixture.wasm'));

const field = (name) => {
  const m = manifestToml.match(new RegExp(`^${name}\\s*=\\s*"([^"]+)"`, 'm'));
  if (!m) throw new Error(`fixture manifest has no ${name}`);
  return m[1];
};
const PLUGIN_ID = field('id');
const VERSION = field('version');

const pluginDir = process.argv[2] ?? './plugins';
const databaseUrl =
  process.env.E2E_DATABASE_URL ||
  'postgres://postgres:postgres@localhost:5432/ethpayserver_e2e';

// The digest is computed here rather than carried alongside the bytes,
// because the host verifies what it reads off disk against what the row
// says. A digest supplied next to the artifact it describes proves nothing;
// one computed from the file is the check working.
const sha256 = createHash('sha256').update(wasm).digest('hex');

const dir = join(pluginDir, PLUGIN_ID);
await mkdir(dir, { recursive: true });
await writeFile(join(dir, `${VERSION}.wasm`), wasm);

const client = new pg.Client({ connectionString: databaseUrl });
await client.connect();
try {
  await client.query(
    `INSERT INTO installed_plugins (id, version, manifest_toml, artifact_sha256, enabled)
     VALUES ($1, $2, $3, $4, true)
     ON CONFLICT (id) DO UPDATE
       SET version = EXCLUDED.version,
           manifest_toml = EXCLUDED.manifest_toml,
           artifact_sha256 = EXCLUDED.artifact_sha256,
           enabled = true,
           disabled_reason = NULL`,
    [PLUGIN_ID, VERSION, manifestToml, sha256],
  );
  console.log(`seeded ${PLUGIN_ID} ${VERSION} (${wasm.length} bytes) into ${dir}`);
} finally {
  await client.end();
}
