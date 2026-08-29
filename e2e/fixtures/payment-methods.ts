import { expect, type Page } from '@playwright/test';
import { createStoreAndOpen, selectStore } from './stores';

/**
 * Account-level xpub (`m/44'/60'/0'`) used by the Rust test suites — see
 * `server/src/api/stores/tests.rs`. Watch-only by construction: an xpub derives
 * receive addresses and cannot spend.
 */
export const TEST_XPUB =
  'xpub6DCoCpSuQZB2jawqnGMEPS63ePKWkwWPH4TU45Q7LPXWuNd8TMtVxRrgjtEshuqpK3mdhaWHPFsBngh5GFZaM6si3yZdUsT8ddYM3PwnATt';

/** Sepolia — the chain the add-method form defaults to. */
export const SEPOLIA = '11155111';

export interface PaymentMethod {
  chainId?: string;
  symbol?: string;
  /** Empty means the chain's native asset. */
  tokenAddress?: string;
  decimals?: string;
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
  const { chainId = SEPOLIA, symbol = 'ETH', tokenAddress = '', decimals = '18' } = method;

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
  await form.getByPlaceholder('xpub...').fill(TEST_XPUB);

  await form.locator('.form-actions .btn-primary').click();
  await expect(form).not.toBeVisible();
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
