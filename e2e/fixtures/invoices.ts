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
