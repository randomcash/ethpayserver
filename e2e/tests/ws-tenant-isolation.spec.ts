/**
 * Live tenant isolation of the status socket (`/ws`).
 *
 * Two ordinary accounts, A and B, each hold one open socket. A creates and owns
 * store S1 only and B creates and owns S2 only, using their own session ids as
 * bearer tokens. Each adds its own payment method and creates its own invoices,
 * so there is no member route and no database insert: creating a store needs
 * only an authenticated user, who becomes its owner, and owning a store is
 * all the socket's gate asks for.
 *
 * Events are produced by expiring real invoices (a one-second expiry; the
 * invoice cleanup service broadcasts `expired`), so nothing is paid and no
 * wallet is touched. They are published one at a time, each only after its
 * intended recipient has received it:
 *
 *   1. an invoice in S2   - B must receive it
 *   2. an invoice in S1   - A must receive it
 *   3. a second one in S2 - B must receive it
 *
 * That order is the point. A held an open socket while (1) was broadcast, so
 * receiving (2) at all, with (1) absent from its frames, shows A's socket was
 * live and still withheld the other tenant's event. The same holds for B
 * across (2) and (3). Only A against (3) has nothing published after it, so
 * that one pair gets a bounded settle window instead.
 *
 * Positive controls come first and are required on both sockets: a dead
 * socket also "received nothing foreign".
 *
 * Local and remote run the same path. They differ only in how the admin token
 * is obtained: a server-admin user created straight in the database locally, the
 * `E2E_API_TOKEN` key remotely. The admin owns nothing under test; it only
 * deletes the two accounts afterwards.
 *
 * If an ordinary account is refused invoice creation on standing, setup has
 * not worked and nothing has been tested: that is reported as NOT VERIFIED,
 * naming the status and body, and never as an isolation result.
 *
 * Off unless `E2E_WS_ISOLATION=true`; once on, missing configuration is a hard
 * failure. The remote run needs `E2E_API_TOKEN`, a server admin key.
 */
import { appendFileSync } from 'node:fs';

import { test, expect } from '@playwright/test';
import { randomBytes } from 'node:crypto';
import { HDKey, generatePrivateKey, privateKeyToAccount } from 'viem/accounts';

import { api, ApiError, V4_UUID } from '../fixtures/api';
import { createUserWithApiKey } from '../fixtures/db';
import {
  expectNothingForeign,
  expectReceived,
  openAuthenticatedSocket,
  type RecordingSocket,
} from '../fixtures/ws-isolation';

const ENABLED = process.env.E2E_WS_ISOLATION === 'true';
const REMOTE = process.env.E2E_REMOTE === 'true';

const CHAIN_ID = 'eip155:11155111';
const MERCHANT_PATH = "m/44'/60'/0'";
/** The cleanup service sweeps on a 60s fallback tick and on block events. */
const EXPIRY_WAIT_MS = 3 * 60_000;
/** For the one pair no later event orders: how long a leak has to show up. */
const SETTLE_MS = 8_000;

interface Account {
  userId: string;
  session: string;
}

function requireEnv(name: string, why: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required when E2E_WS_ISOLATION=true - ${why}`);
  return value;
}

const b64 = (n: number) => randomBytes(n).toString('base64');

/** A throwaway wallet-signup account. Never an admin, never a store owner. */
async function registerOrdinaryAccount(label: string): Promise<Account> {
  const key = privateKeyToAccount(generatePrivateKey());
  const start = await api<{ user_id: string; address: string; challenge_message: string }>(
    '/auth/wallet/new-user/start',
    { method: 'POST', body: { address: key.address, wallet_name: label } },
  );
  const signature = await key.signMessage({ message: start.challenge_message });
  const done = await api<{ session_id: string }>('/auth/wallet/new-user/complete', {
    method: 'POST',
    body: {
      user_id: start.user_id,
      address: start.address,
      signature,
      wallet_name: label,
      kdf_params: { algorithm: 'argon2id', memory_kb: 65536, iterations: 3, parallelism: 4, salt: b64(16) },
      encrypted_symmetric_key: { ciphertext: b64(48), iv: b64(16), mac: b64(32) },
      recovery_verification_hash: b64(32),
      device_name: label,
      device_type: 'api_client',
    },
  });
  return { userId: start.user_id, session: done.session_id };
}

/** Same name shape `scripts/sweep-e2e-stores.mjs` recognises. */
function storeName(offsetMs: number): string {
  return `e2e-synthetic-${new Date(Date.now() + offsetMs).toISOString().replace(/[:.]/g, '-')}`;
}

let adminToken: string | null = null;
let stores: { id: string; session: string }[] = [];
let accounts: Account[] = [];
let sockets: RecordingSocket[] = [];

test.describe('Status socket tenant isolation', () => {
  test.describe.configure({ retries: 0 });
  test.skip(!ENABLED, 'Set E2E_WS_ISOLATION=true to run the live /ws isolation test');

  /**
   * Close the sockets, have each owner archive its own store and the admin
   * delete both ordinary accounts. Archive rather than hard delete, as the synthetic payment spec does. A
   * cleanup failure fails the run only when the test itself passed, so it
   * never buries the isolation result, and it is announced either way.
   */
  test.afterEach(async ({}, testInfo) => {
    for (const s of sockets) s.close();
    sockets = [];

    const token = adminToken;
    const owned = stores;
    const users = accounts;
    stores = [];
    accounts = [];
    if (!token) return;

    const failures: string[] = [];
    for (const { id, session } of owned) {
      await api(`/stores/${id}`, { method: 'DELETE', token: session }).catch((e) =>
        failures.push(`archive store ${id}: ${e}`),
      );
    }
    for (const u of users) {
      await api(`/admin/users/${u.userId}`, { method: 'DELETE', token }).catch((e) =>
        failures.push(`delete account ${u.userId}: ${e}`),
      );
    }
    if (failures.length === 0) return;

    const msg = `ws isolation cleanup left residue: ${failures.join('; ')}`;
    console.log(`::error title=ws isolation cleanup failed::${msg}`);
    if (process.env.GITHUB_STEP_SUMMARY) {
      appendFileSync(process.env.GITHUB_STEP_SUMMARY, `### ❌ ws isolation cleanup failed\n\n${msg}\n`);
    }
    if (testInfo.status === testInfo.expectedStatus) throw new Error(msg);
  });

  test('a socket receives its own stores\' events and no other store\'s', async () => {
    test.setTimeout(4 * EXPIRY_WAIT_MS);
    const token = REMOTE
      ? requireEnv('E2E_API_TOKEN', 'a server admin API key that deletes the test accounts')
      : (await createUserWithApiKey('server_admin')).apiKey;
    adminToken = token;

    const stamp = Date.now().toString(36);
    const a = await registerOrdinaryAccount(`e2e-synthetic-ws-a-${stamp}`);
    accounts.push(a);
    const b = await registerOrdinaryAccount(`e2e-synthetic-ws-b-${stamp}`);
    accounts.push(b);

    const xpub = () => HDKey.fromMasterSeed(randomBytes(32)).derive(MERCHANT_PATH).publicExtendedKey;
    async function makeStore(offsetMs: number, owner: Account): Promise<string> {
      const store = await api<{ id: string }>('/stores', {
        method: 'POST',
        token: owner.session,
        body: { name: storeName(offsetMs) },
      });
      stores.push({ id: store.id, session: owner.session });
      await api(`/stores/${store.id}/payment-methods`, {
        method: 'POST',
        token: owner.session,
        body: { chain_id: CHAIN_ID, token_address: null, asset_symbol: 'ETH', decimals: 18, xpub: xpub() },
      });
      return store.id;
    }
    const s1 = await makeStore(0, a);
    const s2 = await makeStore(1, b);

    // Both sockets are acknowledged before the first event exists.
    const socketA = await openAuthenticatedSocket(a.session);
    sockets.push(socketA);
    const socketB = await openAuthenticatedSocket(b.session);
    sockets.push(socketB);

    async function expireOne(storeId: string, owner: Account): Promise<string> {
      const invoice = await api<{ id: string }>('/invoices', {
        method: 'POST',
        token: owner.session,
        body: {
          store_id: storeId,
          currency: 'ETH',
          amount: '0.0001',
          expiration_seconds: 1,
          metadata: { source: 'ws-tenant-isolation' },
        },
      }).catch((e) => {
        if (e instanceof ApiError && (e.status === 402 || e.status === 403)) {
          throw new Error(
            `NOT VERIFIED - setup refused invoice creation: ${e.status} ${e.body.slice(0, 500)}. ` +
              'This is a setup failure, not an isolation result.',
          );
        }
        throw e;
      });
      expect(invoice.id, 'invoice id is not a v4 UUID').toMatch(V4_UUID);
      return invoice.id;
    }

    const s2First = await expireOne(s2, b);
    const gotB1 = await socketB.waitForInvoice(s2First, EXPIRY_WAIT_MS);
    expect(gotB1.status, 'B was told something other than expiry about its own invoice').toBe('expired');

    const s1Only = await expireOne(s1, a);
    const gotA = await socketA.waitForInvoice(s1Only, EXPIRY_WAIT_MS);
    expect(gotA.status, 'A was told something other than expiry about its own invoice').toBe('expired');

    const s2Second = await expireOne(s2, b);
    await socketB.waitForInvoice(s2Second, EXPIRY_WAIT_MS);

    // The one pair nothing orders: S2's second event against A.
    await new Promise((r) => setTimeout(r, SETTLE_MS));

    // Positive controls, both sockets.
    expectReceived('A', socketA.frames, [s1Only]);
    expectReceived('B', socketB.frames, [s2First, s2Second]);
    // Isolation: nothing outside the socket's own store, in either direction.
    expectNothingForeign('A', socketA.frames, [s1Only]);
    expectNothingForeign('B', socketB.frames, [s2First, s2Second]);

    console.log(
      `A (S1 owner) frames: ${socketA.frames.length}, B (S2 owner) frames: ${socketB.frames.length}; ` +
        `A saw ${s1Only} and nothing of ${s2First}/${s2Second}; B saw ${s2First} and ${s2Second}, nothing of ${s1Only}`,
    );
  });
});
