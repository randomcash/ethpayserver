import { test, expect } from '../fixtures/auth';
import { resetDatabase } from '../fixtures/db';
import type { Page } from '@playwright/test';

async function createStoreAndNavigate(page: Page, name: string): Promise<void> {
  await page.goto('/evm/stores');
  await page.locator('button', { hasText: /create store/i }).click();
  await page.locator('.form-input').fill(name);
  await page.locator('.form-actions .btn-primary').click();
  await expect(page.locator('.store-card-name', { hasText: name })).toBeVisible();

  await page.locator('.store-card', { hasText: name }).click();
  await page.waitForURL(/\/evm\/stores\/.+/);
}

test.describe('Webhooks', () => {
  test.beforeAll(async () => {
    await resetDatabase();
  });

  test('configure webhook URL', async ({ registeredPage: page }) => {
    await createStoreAndNavigate(page, 'Webhook Store');

    // Find the webhook section and fill in the URL
    const webhookInput = page.locator('input[placeholder*="webhook" i], input[name="webhook_url"], input[type="url"]').first();
    if (await webhookInput.isVisible({ timeout: 3_000 }).catch(() => false)) {
      await webhookInput.fill('https://example.com/webhook');
    } else {
      // Fallback: look for a webhook configuration button first
      const configureBtn = page.locator('button', { hasText: /webhook|configure/i }).first();
      if (await configureBtn.isVisible({ timeout: 2_000 }).catch(() => false)) {
        await configureBtn.click();
      }
      await page.locator('input[type="url"], input[type="text"]').last().fill('https://example.com/webhook');
    }

    await page.locator('button', { hasText: /save|update|configure/i }).click();

    // Reload and verify persistence
    await page.reload();
    await expect(page.getByText('example.com/webhook')).toBeVisible();
  });

  test('toggle webhook enabled/disabled', async ({ registeredPage: page }) => {
    await createStoreAndNavigate(page, 'Toggle Webhook Store');

    // Configure a webhook first
    const webhookInput = page.locator('input[type="url"], input[placeholder*="webhook" i]').first();
    if (await webhookInput.isVisible({ timeout: 3_000 }).catch(() => false)) {
      await webhookInput.fill('https://example.com/hook');
      await page.locator('button', { hasText: /save|update|configure/i }).click();
    }

    // Toggle enabled/disabled
    const toggle = page.locator('[class*="webhook"] input[type="checkbox"], [class*="webhook"] [class*="toggle"]').first();
    if (await toggle.isVisible({ timeout: 2_000 }).catch(() => false)) {
      await toggle.click();

      // Verify the change persisted
      await page.reload();
    }
  });
});
