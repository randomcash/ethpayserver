/**
 * A mock EIP-1193 wallet provider for tests.
 *
 * Playwright's Chromium ships no wallet extension, so `window.ethereum` is
 * undefined and every wallet code path in the client is unreachable. This
 * injects a provider backed by a real secp256k1 key, so the signatures it
 * produces are ones the server actually verifies — the point is to exercise
 * the real challenge/verify round trip, not to stub past it.
 *
 * The key is generated per installation. Accounts are keyed on wallet address
 * server-side, so a shared address across tests collides the moment two tests
 * register; `testXpubFor` generates per-account for the same reason.
 */

import { expect, type Page } from '@playwright/test';
import { generatePrivateKey, privateKeyToAccount } from 'viem/accounts';
import { finishRegistration, type RecoveryCredentials } from './auth';

/** Error code for "user rejected the request" (EIP-1193). */
export const USER_REJECTED = 4001;

export interface MockWallet {
  /** The checksummed address the provider reports. */
  address: string;
  /** Make the next `personal_sign` reject the way a user declining does. */
  rejectNextSignature(): Promise<void>;
  /** Number of `personal_sign` calls so far — proves a fresh challenge per login. */
  signatureCount(): Promise<number>;
}

/**
 * Install a mock wallet into `page` before any app code runs.
 *
 * Must be called before the first navigation: `addInitScript` only affects
 * documents created after it is registered, so installing it after `goto`
 * leaves the already-loaded page with no provider.
 */
export async function installMockWallet(page: Page): Promise<MockWallet> {
  const account = privateKeyToAccount(generatePrivateKey());

  // Signing happens in Node, not in the page: viem is not bundled into the
  // client, and exposing a signing function keeps the private key out of the
  // browser context entirely.
  const state = { rejectNext: false, signatures: 0 };

  await page.exposeFunction(
    '__mockWalletSign',
    async (message: string): Promise<{ ok: true; signature: string } | { ok: false; code: number; message: string }> => {
      if (state.rejectNext) {
        state.rejectNext = false;
        return { ok: false, code: USER_REJECTED, message: 'User rejected the request.' };
      }
      state.signatures += 1;
      // The client passes the challenge as a plain UTF-8 string, so sign it as
      // one. Hex-decoding here would sign different bytes than the server
      // hashed when it issued the challenge, and every login would fail
      // verification for reasons that look like a server bug.
      return { ok: true, signature: await account.signMessage({ message }) };
    },
  );

  await page.addInitScript(
    ({ address, rejectedCode }) => {
      const provider = {
        isMetaMask: true,
        async request({ method, params }: { method: string; params?: unknown[] }) {
          switch (method) {
            case 'eth_requestAccounts':
            case 'eth_accounts':
              return [address];

            case 'personal_sign': {
              // EIP-191 personal_sign params are [message, address].
              const message = (params ?? [])[0] as string;
              const signed = await (
                window as unknown as {
                  __mockWalletSign(m: string): Promise<
                    { ok: true; signature: string } | { ok: false; code: number; message: string }
                  >;
                }
              ).__mockWalletSign(message);

              if (!signed.ok) {
                // Shaped like a real provider rejection: the client reads
                // `.code` to tell a decline apart from a failure, so throwing a
                // bare Error would test the wrong branch.
                const err = new Error(signed.message) as Error & { code: number };
                err.code = signed.code;
                throw err;
              }
              return signed.signature;
            }

            case 'eth_chainId':
              return '0x1';

            default:
              const err = new Error(`Unsupported method: ${method}`) as Error & { code: number };
              err.code = 4200;
              throw err;
          }
        },
        // Wallets are event emitters; the client may subscribe even though it
        // does not act on them yet. No-ops keep it from throwing.
        on() {},
        removeListener() {},
      };

      Object.defineProperty(window, 'ethereum', {
        value: provider,
        writable: true,
        configurable: true,
      });

      // Announced both ways on purpose. The client reads `window.ethereum`
      // today (legacy single-provider injection), but the agreed direction is
      // EIP-6963-first discovery. Announcing both means this fixture keeps
      // working across that migration instead of needing a rewrite, and will
      // actually prove the new path when it lands.
      const info = {
        uuid: '00000000-0000-4000-8000-000000000001',
        name: 'Mock Wallet',
        icon: 'data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciLz4=',
        rdns: 'cash.random.mockwallet',
      };
      const announce = () =>
        window.dispatchEvent(
          new CustomEvent('eip6963:announceProvider', {
            detail: Object.freeze({ info, provider }),
          }),
        );
      window.addEventListener('eip6963:requestProvider', announce);
      announce();

      void rejectedCode;
    },
    { address: account.address, rejectedCode: USER_REJECTED },
  );

  return {
    address: account.address,
    async rejectNextSignature() {
      state.rejectNext = true;
    },
    async signatureCount() {
      return state.signatures;
    },
  };
}

/**
 * Register a new account through the wallet tab.
 *
 * Returns the recovery material, exactly as the passkey `register` does. A
 * wallet account still gets a phrase — the wallet is how you log in, not how
 * you recover.
 */
export async function registerWithWallet(
  page: Page,
  wallet: MockWallet,
): Promise<RecoveryCredentials> {
  await page.goto('/register');
  await page.locator('.ps-auth-tab', { hasText: /wallet/i }).click();
  await page.locator('.ps-wallet-button').click();

  // The connect button resolving is not the same as the server having accepted
  // the signature. Waiting for the connected address to render means the
  // challenge round trip actually completed before the recovery step is driven.
  const shown = page.locator('.ps-wallet-address');
  await expect(shown).toBeVisible({ timeout: 20_000 });

  // The page truncates the address for display, so compare on the ends rather
  // than equality — but do compare: rendering *an* address proves the flow ran,
  // not that it bound the account to the key that actually signed.
  const text = ((await shown.textContent()) ?? '').toLowerCase();
  const addr = wallet.address.toLowerCase();
  expect(text).toContain(addr.slice(0, 6));
  expect(text).toContain(addr.slice(-4));

  return finishRegistration(page);
}

/** Log an existing wallet account back in through the wallet tab. */
export async function loginWithWallet(page: Page): Promise<void> {
  await page.goto('/login');
  await page.locator('.ps-auth-tab', { hasText: /wallet/i }).click();
  await page.locator('.ps-wallet-button').click();
  await page.waitForURL(/\/(evm)?$/, { timeout: 20_000 });
}
