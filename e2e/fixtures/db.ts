import { Client } from 'pg';

const DATABASE_URL =
  process.env.E2E_DATABASE_URL || 'postgres://postgres:postgres@localhost:5432/ethpayserver_e2e';

const TABLES_TO_TRUNCATE = [
  'api_keys',
  'payment_events',
  'payments',
  'watched_addresses',
  'payment_options',
  'invoices',
  'store_payment_methods',
  'store_webhooks',
  'user_stores',
  'store_roles',
  'stores',
  'discoverable_authentication_challenges',
  'wallet_challenges',
  'passkey_authentication_challenges',
  'passkey_registration_challenges',
  'wallet_credentials',
  'passkey_credentials',
  'sessions',
  'devices',
  'users',
];

export async function resetDatabase(): Promise<void> {
  if (process.env.E2E_SKIP_DB_RESET || process.env.E2E_REMOTE) {
    return;
  }
  const client = new Client({ connectionString: DATABASE_URL });
  await client.connect();
  try {
    await client.query(`TRUNCATE TABLE ${TABLES_TO_TRUNCATE.join(', ')} CASCADE`);
  } finally {
    await client.end();
  }
}
