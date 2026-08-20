import { expect, type Locator, type Page } from '@playwright/test';

/**
 * The Create Invoice trigger in the header action bar.
 *
 * Scoped to `.main-header-actions` because "Create Invoice" appears up to three
 * times on an authenticated page — the header trigger, the modal's own submit
 * button in `.modal-footer`, and the invoices page's empty-state button — so a
 * bare `button` filtered on the text is a strict-mode violation rather than a
 * trigger.
 */
export function createInvoiceTrigger(page: Page): Locator {
  return page.locator('.main-header-actions button', { hasText: /create invoice/i });
}

/** The Create Invoice modal. Mounted always; shown by toggling `display`. */
export function createInvoiceModal(page: Page): Locator {
  return page.locator('.modal-overlay');
}

/** Open the Create Invoice modal from the header and wait for it. */
export async function openCreateInvoiceModal(page: Page): Promise<Locator> {
  await createInvoiceTrigger(page).click();
  const modal = createInvoiceModal(page);
  await expect(modal).toBeVisible();
  return modal;
}

/** Submit the open Create Invoice modal. */
export async function submitCreateInvoice(page: Page): Promise<void> {
  await createInvoiceModal(page).locator('.modal-footer button[type="submit"]').click();
}

/** Rows of the invoice list. `.invoices-cards` is the mobile-only twin. */
export function invoiceRows(page: Page): Locator {
  return page.locator('.invoices-table tbody tr');
}

/**
 * Create an invoice for the currently-selected store.
 *
 * The modal has no store picker — it bills whatever the sidebar has selected —
 * so callers must `selectStore` first. Fields are addressed by their ids
 * (`#ci-amount`), not by type: the amount is `type="text"` with
 * `inputmode="decimal"`, so the old `input[type="number"]` never matched it.
 */
export async function createInvoice(
  page: Page,
  amount: string,
  currency = 'ETH',
): Promise<void> {
  const modal = await openCreateInvoiceModal(page);
  // ETH rather than the form's USD default: a fiat invoice needs a live
  // exchange rate, and the server answers 503 `rate_stale` when its rate feed
  // is behind, which makes the test depend on Kraken/CoinGecko being reachable.
  // Priced in the same asset it is paid in, the flow needs no rate at all.
  await modal.locator('#ci-currency').selectOption(currency);
  await modal.locator('#ci-amount').fill(amount);
  await submitCreateInvoice(page);
  // On success the modal closes and navigates to the new invoice's detail page
  // — it does not return to the list, so that navigation is the completion
  // signal. A modal that stays open is a rejected create; its error banner is
  // in `.form-alert-error`.
  await page.waitForURL(/\/evm\/invoices\/.+/);
}

/**
 * Close the open Create Invoice modal via its header close button.
 *
 * The specs used to try `.modal-close, button[aria-label="Close"]` and silently
 * fall back to clicking the overlay corner when that missed — the real control
 * is the icon button in `.modal-header`.
 */
export async function closeCreateInvoiceModal(page: Page): Promise<void> {
  await createInvoiceModal(page).locator('.modal-header .btn-icon').click();
  await expect(createInvoiceModal(page)).not.toBeVisible();
}
