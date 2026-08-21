import { test, expect } from '../fixtures/auth';
import { resetDatabase } from '../fixtures/db';
import { createStoreAndOpen } from '../fixtures/stores';
import { configureWebhook, openWebhooksTab, webhookForm } from '../fixtures/webhooks';

const WEBHOOK_URL = 'https://example.com/webhook';

test.describe('Webhooks', () => {
  test.beforeAll(async () => {
    await resetDatabase();
  });

  test('configure webhook URL', async ({ registeredPage: page }) => {
    await createStoreAndOpen(page, 'Webhook Store');
    await openWebhooksTab(page);

    await configureWebhook(page, WEBHOOK_URL);

    await expect(page.locator('.webhook-url')).toHaveText(WEBHOOK_URL);

    // and it survives a round trip to the server
    await page.reload();
    await openWebhooksTab(page);
    await expect(page.locator('.webhook-url')).toHaveText(WEBHOOK_URL);
  });

  test('toggle webhook enabled/disabled', async ({ registeredPage: page }) => {
    await createStoreAndOpen(page, 'Toggle Webhook Store');
    await openWebhooksTab(page);
    await configureWebhook(page, 'https://example.com/hook');

    // The endpoint card's badge reads Active/Disabled (the checkbox in the form
    // is the one labelled "Enabled").
    const status = page.locator('.detail-card-header .badge').first();
    await expect(status).toHaveText('Active');

    // Re-open the form and clear the Enabled checkbox
    await page.locator('button', { hasText: 'Edit endpoint' }).click();
    const form = webhookForm(page);
    await form.locator('input[type="checkbox"]').uncheck();
    await form.locator('.btn-primary', { hasText: 'Save' }).click();

    await expect(status).toHaveText('Disabled');

    await page.reload();
    await openWebhooksTab(page);
    await expect(page.locator('.detail-card-header .badge').first()).toHaveText('Disabled');
  });
});
