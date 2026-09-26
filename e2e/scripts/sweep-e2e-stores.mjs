#!/usr/bin/env node
/**
 * Remove the stores the synthetic-payment test left behind.
 *
 * The spec creates `e2e-synthetic-<timestamp>` on every scheduled run and for
 * a long time deleted none of them — testnet gained one per day. The fix in
 * `tests/synthetic-payment.spec.ts` stops the bleeding; this clears what had
 * already piled up, and mops up after any run that dies before its cleanup hook.
 *
 *   E2E_API_URL=https://testnet.random.cash E2E_REMOTE=true \
 *   E2E_API_TOKEN=ak_... node scripts/sweep-e2e-stores.mjs [--execute]
 *
 * Dry run by default: it lists what it would remove and changes nothing. Only
 * `--execute` issues deletes, because this runs against a live server and the
 * rows are real.
 *
 * Two things worth knowing before reading the output:
 *
 * - `GET /stores` returns only the stores the token's own user can see, so this
 *   can never reach another account's stores however wrong the pattern goes.
 * - This calls `DELETE /admin/stores/{id}` — `hard_delete_store` in
 *   `server/src/api/admin/mod.rs` — not the self-service `DELETE /stores/{id}`,
 *   which only archives. The admin route actually removes the row, its
 *   invoices and its payments, which is the point: an archived store is still
 *   a row the daily count keeps growing by. `E2E_API_TOKEN` must be a
 *   `server_admin` token for this to work, same requirement as
 *   `sweep-e2e-accounts.mjs`. The endpoint independently refuses any store
 *   whose name is not this exact `e2e-synthetic-<ISO timestamp>` shape, so a
 *   loose pattern here can widen what this script *lists* but never what it
 *   can actually delete.
 */

/**
 * Anchored, and deliberately tighter than the `e2e-synthetic-%` the ticket
 * describes: the suffix must be the exact ISO stamp the spec builds
 * (`new Date().toISOString().replace(/[:.]/g, '-')`, e.g.
 * `2026-08-27T17-29-33-596Z`). A prefix match alone would take a store someone
 * named `e2e-synthetic-scratch` by hand, and this deletes on a live server.
 */
const SYNTHETIC_STORE_NAME = /^e2e-synthetic-\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}-\d{3}Z$/;
/** Reported separately rather than swept: close enough to be worth a human look. */
const NEAR_MISS_PREFIX = 'e2e-synthetic-';

function requireEnv(name, why) {
  const v = process.env[name];
  if (!v) {
    console.error(`${name} is required — ${why}`);
    process.exit(1);
  }
  return v;
}

const execute = process.argv.slice(2).includes('--execute');

const apiUrl = requireEnv(
  'E2E_API_URL',
  'the server to sweep, e.g. https://testnet.random.cash',
).replace(/\/$/, '');
const token = requireEnv(
  'E2E_API_TOKEN',
  'server_admin API key (ak_...) — hard-deleting a store is an admin action',
);
// Same rule as fixtures/api.ts: the deployed client's nginx proxies /api/ to the
// backend and strips the prefix, so a remote base URL needs it and a direct one
// does not. Getting it wrong 404s loudly rather than silently finding nothing.
const prefix = (
  process.env.E2E_API_PREFIX !== undefined
    ? process.env.E2E_API_PREFIX
    : process.env.E2E_REMOTE === 'true'
      ? '/api'
      : ''
).replace(/\/$/, '');

async function api(path, method = 'GET') {
  const resp = await fetch(`${apiUrl}${prefix}${path}`, {
    method,
    headers: { Authorization: `Bearer ${token}` },
  });
  const text = await resp.text();
  if (!resp.ok) throw new Error(`${method} ${path} → ${resp.status}: ${text.slice(0, 300)}`);
  return text ? JSON.parse(text) : undefined;
}

console.log(`sweeping ${apiUrl}${prefix}`);
console.log(execute ? 'MODE: execute\n' : 'MODE: dry run (pass --execute to delete)\n');

const stores = await api('/stores');

// Every matching store is residue, archived or not — an archive from a run
// before this script hard-deleted is exactly the pile this backfill exists
// to clear, not something to treat as already handled.
const matched = stores.filter((s) => SYNTHETIC_STORE_NAME.test(s.name));
const nearMisses = stores.filter(
  (s) => !SYNTHETIC_STORE_NAME.test(s.name) && s.name.startsWith(NEAR_MISS_PREFIX),
);

for (const store of matched) {
  console.log(`  ${execute ? 'delete' : 'would delete'}  ${store.id}  ${store.name}`);
}

let failed = 0;
if (execute) {
  for (const store of matched) {
    try {
      await api(`/admin/stores/${store.id}`, 'DELETE');
    } catch (err) {
      // Keep going and fail at the end: one 403/409 must not strand the rest.
      console.error(`  FAILED  ${store.id}  ${store.name} — ${err.message}`);
      failed++;
    }
  }
}

console.log(`\n${stores.length} store(s) visible to this token, ${matched.length} matching ${SYNTHETIC_STORE_NAME}.`);
if (nearMisses.length > 0) {
  console.log(
    `\n${nearMisses.length} store(s) start with "${NEAR_MISS_PREFIX}" but do not match the ` +
      `timestamp shape and were left alone — check them by hand:`,
  );
  for (const s of nearMisses) console.log(`  skipped  ${s.id}  ${s.name}`);
}
if (!execute && matched.length > 0) console.log('\nRe-run with --execute to delete.');
if (failed > 0) {
  console.error(`\n${failed} deletion(s) failed.`);
  process.exit(1);
}
