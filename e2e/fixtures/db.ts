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

/**
 * A user with an API key, created directly in the database.
 *
 * Registration goes through WebAuthn in a browser, which is the right way to
 * test registration and a poor way to get a credential for a suite that is
 * testing something else. Inserting the row skips a virtual authenticator, a
 * page load and a ceremony, none of which this returns anything about.
 *
 * The three encrypted columns are `NOT NULL` and hold client-side material
 * the server never reads back for an API-key request, so a placeholder is
 * honest here rather than lazy - a real value would suggest this account can
 * log in, and it cannot.
 */
export async function createUserWithApiKey(
  role: 'user' | 'server_admin' = 'user',
): Promise<{ userId: string; apiKey: string }> {
  const crypto = await import('node:crypto');
  // `ak_` because that is the shape `validate_api_key` looks for, and a key
  // that does not start with it fails for a reason that reads as "wrong
  // credential" rather than "wrong prefix".
  const apiKey = `ak_e2e_${crypto.randomBytes(18).toString('hex')}`;
  // SHA-256, and it has to be: `auth::api::api_keys` hashes the key exactly
  // this way, so anything else here produces a credential `validate_api_key`
  // can never match. Not a password hash and not meant to be - the value is
  // 144 bits of randomness this process just generated, so there is no
  // guessing to slow down and nothing for a work factor to buy. Code scanning
  // flags it on the identifier's name; the alert is dismissed as a false
  // positive with that reason.
  const keyHash = crypto.createHash('sha256').update(apiKey).digest('hex');

  const client = new Client({ connectionString: DATABASE_URL });
  await client.connect();
  try {
    const { rows } = await client.query(
      // Both JSONB columns get '{}' and the VARCHAR one gets a string. That
      // is not cosmetic: `encrypted_symmetric_key` is JSONB NOT NULL, so a
      // bare word here is `invalid input syntax for type json` - thrown from
      // this helper, which every plugin test calls in `beforeAll`. The whole
      // file then reports one failure at 0ms and eight skips, which reads as a
      // broken test rather than a broken fixture.
      //
      // The values are never decrypted. Nothing in these tests logs in with a
      // password; they authenticate with the API key created below.
      `INSERT INTO users (email, kdf_params, encrypted_symmetric_key,
                          recovery_verification_hash, role)
       VALUES ($1, '{}', '{}', 'e2e-placeholder', $2)
       RETURNING id`,
      [`e2e-${crypto.randomBytes(6).toString('hex')}@example.test`, role],
    );
    const userId = rows[0].id as string;

    await client.query(
      `INSERT INTO api_keys (user_id, name, key_hash, key_prefix, is_active)
       VALUES ($1, 'e2e', $2, $3, true)`,
      [userId, keyHash, apiKey.slice(0, 12)],
    );
    return { userId, apiKey };
  } finally {
    await client.end();
  }
}
