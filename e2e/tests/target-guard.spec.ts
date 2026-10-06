/**
 * The refusal must fire before any write. These feed the guard an explicit
 * environment, so they run with no server and no network.
 */
import { test, expect } from '@playwright/test';

import { assertSafeTarget, LIVE_SERVICE_PORT, OPT_IN_VARIABLE } from '../fixtures/target-guard';

const live = `http://localhost:${LIVE_SERVICE_PORT}`;

test('the live service port is refused in local mode, even on loopback', () => {
  expect(() => assertSafeTarget({ E2E_API_URL: live })).toThrow(/live service's port/);
  expect(() => assertSafeTarget({ E2E_API_URL: `http://127.0.0.1:${LIVE_SERVICE_PORT}` })).toThrow(
    /live service's port/,
  );
});

test('the opt-in does not unlock the live service port', () => {
  expect(() => assertSafeTarget({ E2E_API_URL: live, [OPT_IN_VARIABLE]: 'true' })).toThrow(
    /live service's port/,
  );
});

test('a non-loopback host or an out-of-range port needs the opt-in', () => {
  for (const url of ['https://testnet.example.com', 'http://10.0.0.5:3000', 'http://localhost:4000']) {
    expect(() => assertSafeTarget({ E2E_API_URL: url })).toThrow(/refusing to run in local mode/);
    expect(() => assertSafeTarget({ E2E_API_URL: url, [OPT_IN_VARIABLE]: 'true' })).not.toThrow();
  }
});

test('the frontend URL is guarded too', () => {
  expect(() => assertSafeTarget({ E2E_BASE_URL: 'https://testnet.example.com' })).toThrow();
});

test('E2E_REMOTE=false is still local mode', () => {
  expect(() => assertSafeTarget({ E2E_REMOTE: 'false', E2E_API_URL: live })).toThrow();
});

test('the harness defaults and CI targets pass', () => {
  expect(() => assertSafeTarget({})).not.toThrow();
  expect(() =>
    assertSafeTarget({ E2E_API_URL: 'http://localhost:3000', E2E_BASE_URL: 'http://localhost:8080' }),
  ).not.toThrow();
});

test('remote mode is untouched', () => {
  expect(() =>
    assertSafeTarget({ E2E_REMOTE: 'true', E2E_API_URL: 'https://testnet.example.com' }),
  ).not.toThrow();
});
