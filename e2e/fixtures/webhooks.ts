import { expect, type Page } from '@playwright/test';

/**
 * Open the "Webhooks" tab of an already-open store detail page.
 *
 * Like payment methods, webhooks live on a tab, so nothing in this UI exists
 * until it is selected.
 */
export async function openWebhooksTab(page: Page): Promise<void> {
  await page.locator('.store-tabs .store-tab', { hasText: 'Webhooks' }).click();
  await expect(page.locator('.store-tab-webhooks')).toBeVisible();
}

/**
 * Set the store's webhook endpoint from the Webhooks tab.
 *
 * The form is hidden until "Configure webhook" is clicked — the old spec looked
 * for the URL input straight away, missed, and fell through to a chain of
 * `if (visible)` branches that asserted nothing when they missed too.
 */
export function webhookForm(page: Page) {
  // Anchored on the field it contains: `.detail-card` nests, so filtering the
  // cards by their heading text matches the outer card as well as the form.
  return page
    .locator('.detail-card')
    .filter({ has: page.getByPlaceholder('https://example.com/webhooks/payments') });
}

export async function configureWebhook(page: Page, url: string): Promise<void> {
  await page.locator('button', { hasText: 'Configure webhook' }).click();

  const form = webhookForm(page);
  await expect(form).toBeVisible();
  await form.getByPlaceholder('https://example.com/webhooks/payments').fill(url);
  await form.locator('.btn-primary', { hasText: 'Save' }).click();

  await expect(form).not.toBeVisible();
}
