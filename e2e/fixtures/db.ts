import { Client } from 'pg';

const DATABASE_URL =
  process.env.E2E_DATABASE_URL || 'postgres://postgres:postgres@localhost:5432/ethpayserver_e2e';

// `payouts.store_id` and `refunds.store_id` reference `stores(id)` with no ON
// DELETE action, so they must be cleared before the row-level `DELETE FROM
// stores` below or it raises a foreign-key violation out of `beforeAll` and
// errors the whole spec file. TRUNCATE ... CASCADE used to absorb that.
//
// `users`, `stores` and `store_roles` are deliberately absent — they are
// cleared with row-level DELETEs below.
//
// `store_roles` holds the four global default roles (`store_id IS NULL`) seeded
// by migration 20241214000001, and `create_store` looks up 'Owner' there for
// every store it creates. TRUNCATE dropped them, so from the first reset onward
// every store creation answered HTTP 500 and every test needing a store failed.
//
// Listing the table is not the only way to lose it: TRUNCATE ... CASCADE
// truncates referencing tables wholesale and follows the chain, so truncating
// `users` reaches `stores` (via `stores.owner_id`) and `stores` reaches
// `store_roles`. Row-level DELETEs cascade per row instead, which takes the
// per-store roles and leaves the defaults.
const TABLES_TO_TRUNCATE = [
  'api_keys',
  'payouts',
  'refunds',
  'payment_events',
  'payments',
  'watched_addresses',
  'payment_options',
  'invoices',
  'store_payment_methods',
  'store_webhooks',
  'user_stores',
  'discoverable_authentication_challenges',
  'wallet_challenges',
  'passkey_authentication_challenges',
  'passkey_registration_challenges',
  'wallet_credentials',
  'passkey_credentials',
  'sessions',
  'devices',
];

export async function resetDatabase(): Promise<void> {
  // E2E_SKIP_DB_RESET stays a presence flag (README documents it as set/unset);
  // E2E_REMOTE is compared strictly so `E2E_REMOTE=false` cannot mean remote.
  if (process.env.E2E_SKIP_DB_RESET || process.env.E2E_REMOTE === 'true') {
    return;
  }
  const client = new Client({ connectionString: DATABASE_URL });
  await client.connect();
  try {
    await client.query(`TRUNCATE TABLE ${TABLES_TO_TRUNCATE.join(', ')} CASCADE`);
    await client.query('DELETE FROM stores');
    await client.query('DELETE FROM users');
    await client.query('DELETE FROM store_roles WHERE store_id IS NOT NULL');
  } finally {
    await client.end();
  }
}
