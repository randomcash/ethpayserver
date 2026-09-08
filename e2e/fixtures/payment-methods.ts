import { randomBytes } from 'node:crypto';

import { expect, type Page } from '@playwright/test';
import { HDKey } from 'viem/accounts';

import { createStoreAndOpen, selectStore } from './stores';

/** Account-level path the server expects an xpub at (`evm/src/wallet.rs`). */
const ACCOUNT_PATH = "m/44'/60'/0'";

/**
 * One xpub per account, generated, rather than one constant for the whole suite.
 *
 * This used to be a shared `TEST_XPUB` constant, and RCS-234 made that invalid:
 * an xpub may now belong to exactly one account, because two accounts deriving
 * from one key issue the same addresses to different merchants' customers.
 * `registeredPage` creates a fresh account per test, so from the second test
 * onwards every add-method call got `409 Conflict` and the form never closed.
 *
 * It did not show up as a failure. Playwright restarts the worker after a
 * failed test, `beforeAll` re-runs `resetDatabase()`, and the retry - now the
 * only account in an empty database - passes. Five tests were reported "flaky"
 * while in fact failing every time, for a real and entirely reproducible
 * reason. Do not put the constant back.
 *
 * Keyed by `Page`, not per call, so one account's methods share one key: that
 * is the realistic shape (paste the same xpub for ETH and USDC) and it is what
 * exercises the one-counter-per-key path RCS-234 exists for. A WeakMap, so
 * pages are not retained after their test ends.
 *
 * Watch-only by construction: an xpub derives receive addresses and cannot
 * spend. The seed is random per account and never leaves the test process.
 */
const xpubByPage = new WeakMap<Page, string>();

export function testXpubFor(page: Page): string {
  const existing = xpubByPage.get(page);
  if (existing) return existing;

  const xpub = HDKey.fromMasterSeed(randomBytes(64)).derive(ACCOUNT_PATH).publicExtendedKey;
  xpubByPage.set(page, xpub);
  return xpub;
}

/** Sepolia — the chain the add-method form defaults to. */
export const SEPOLIA = '11155111';

export interface PaymentMethod {
  chainId?: string;
  symbol?: string;
  /** Empty means the chain's native asset. */
  tokenAddress?: string;
  decimals?: string;
  /**
   * Override the account's key. Only for tests that are *about* the xpub -
   * anything else should take the per-account default, or it risks
   * reintroducing the cross-account collision described above.
   */
  xpub?: string;
}

/**
 * Open the "Payment Methods" tab of an already-open store detail page.
 *
 * Payment methods live on a tab, so the add-method button does not exist until
 * it is selected — the specs used to look for the button straight after opening
 * the store and waited out a 30s timeout. Note the button reads "Add method";
 * "Add Payment Method" is the heading of the form it opens, which is what the
 * old `/add.*payment.*method/i` was matching.
 */
export async function openPaymentMethodsTab(page: Page): Promise<void> {
  await page.locator('.store-tabs .store-tab', { hasText: 'Payment Methods' }).click();
  await expect(page.locator('.store-tab-payment-methods')).toBeVisible();
}

/** Add a payment method to the open store, from its Payment Methods tab. */
export async function addPaymentMethod(page: Page, method: PaymentMethod = {}): Promise<void> {
  const {
    chainId = SEPOLIA,
    symbol = 'ETH',
    tokenAddress = '',
    decimals = '18',
    xpub = testXpubFor(page),
  } = method;

  // "Add method" *toggles* `show_create_form`, so clicking it blindly closes an
  // already-open form (e.g. on a retry after a failed create) and the wait below
  // then times out reporting "form not visible". Only click when it is shut.
  const form = page.locator('.detail-card', { hasText: 'Add Payment Method' });
  if (!(await form.isVisible())) {
    await page.locator('.store-tab-payment-methods button', { hasText: /add method/i }).click();
  }
  await expect(form).toBeVisible();

  await form.locator('.form-select').selectOption(chainId);
  await form.getByPlaceholder('ETH').fill(symbol);
  if (tokenAddress) {
    await form.getByPlaceholder(/leave empty for native/i).fill(tokenAddress);
  }
  await form.locator('input[type="number"]').fill(decimals);
  await form.getByPlaceholder('xpub...').fill(xpub);

  await form.locator('.form-actions .btn-primary').click();

  // Read the error only AFTER the wait has run out, never as an argument to the
  // assertion. `expect(form, await ...)` evaluates the message first, which
  // snapshots the form the instant the click returns - before the server has
  // answered - so a 409 that renders a second later is missed and the report
  // says "showed no error". It also costs a second per call on the happy path,
  // waiting for an error locator that will never resolve because the form
  // closed.
  try {
    await expect(form).not.toBeVisible();
  } catch (err) {
    throw new Error(`${await addMethodFailure(form)}\n\n${(err as Error).message}`);
  }
}

/**
 * Explain a form that would not close, using whatever error it is showing.
 *
 * Called only on the failure path, once the assertion above has already given
 * the response every chance to arrive. Without this the report is "locator
 * resolved to <div class=detail-card>" nine times over, and the actual cause is
 * only visible by downloading the trace.
 */
async function addMethodFailure(form: ReturnType<Page['locator']>): Promise<string> {
  const text = await form
    .locator('.form-error, .error-message, .alert-error')
    .first()
    .textContent({ timeout: 1_000 })
    .catch(() => null);

  return text?.trim()
    ? `the add-method form stayed open: ${text.trim()}`
    : 'the add-method form stayed open and showed no error';
}

/**
 * Create a store, give it one native payment method, and leave it selected.
 *
 * The invoice API rejects a store with no enabled payment method
 * (`no_payment_methods`), so every invoice test needs this much setup.
 */
export async function createStoreReadyForInvoices(page: Page, name: string): Promise<void> {
  await createStoreAndOpen(page, name);
  await openPaymentMethodsTab(page);
  await addPaymentMethod(page);
  await selectStore(page, name);
}
