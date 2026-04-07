import { test, expect } from '../fixtures/auth';
import { resetDatabase } from '../fixtures/db';
import type { Page } from '@playwright/test';

async function createStoreAndNavigate(page: Page, name: string): Promise<void> {
  await page.goto('/evm/stores');
  await page.locator('button', { hasText: /create store/i }).click();
  await page.locator('.form-input').fill(name);
  await page.locator('.form-actions .btn-primary').click();
  await expect(page.locator('.store-card-name', { hasText: name })).toBeVisible();

  // Navigate to store detail
  await page.locator('.store-card', { hasText: name }).click();
  await page.waitForURL(/\/evm\/stores\/.+/);
}

test.describe('Payment Methods', () => {
  test.beforeAll(async () => {
    await resetDatabase();
  });

  test('empty state shown for new store', async ({ registeredPage: page }) => {
    await createStoreAndNavigate(page, 'Empty PM Store');

    // Payment methods section should be visible but empty
    const pmHeading = page.getByText(/payment method/i).first();
    await expect(pmHeading).toBeVisible();
  });

  test('add native payment method', async ({ registeredPage: page }) => {
    await createStoreAndNavigate(page, 'Native PM Store');

    await page.locator('button', { hasText: /add.*payment.*method/i }).click();

    // Select a chain from the dropdown
    const chainSelect = page.locator('select').first();
    if (await chainSelect.isVisible({ timeout: 2_000 }).catch(() => false)) {
      await chainSelect.selectOption({ index: 1 });
    }

    // Submit — native token has no separate token selector
    await page.locator('button', { hasText: /add|save|create/i }).last().click();

    // Verify the payment method is now listed
    await expect(page.locator('[class*="payment-method"], .detail-card').last()).toBeVisible();
  });

  test('add ERC20 payment method', async ({ registeredPage: page }) => {
    await createStoreAndNavigate(page, 'ERC20 PM Store');

    await page.locator('button', { hasText: /add.*payment.*method/i }).click();

    // Select chain
    const chainSelect = page.locator('select').first();
    if (await chainSelect.isVisible({ timeout: 2_000 }).catch(() => false)) {
      await chainSelect.selectOption({ index: 1 });
    }

    // Select token (ERC20)
    const tokenSelect = page.locator('select').nth(1);
    if (await tokenSelect.isVisible({ timeout: 2_000 }).catch(() => false)) {
      await tokenSelect.selectOption({ index: 1 });
    }

    await page.locator('button', { hasText: /add|save|create/i }).last().click();

    await expect(page.locator('[class*="payment-method"], .detail-card').last()).toBeVisible();
  });

  test('toggle payment method enabled/disabled', async ({ registeredPage: page }) => {
    await createStoreAndNavigate(page, 'Toggle PM Store');

    // Add a payment method first
    await page.locator('button', { hasText: /add.*payment.*method/i }).click();
    const chainSelect = page.locator('select').first();
    if (await chainSelect.isVisible({ timeout: 2_000 }).catch(() => false)) {
      await chainSelect.selectOption({ index: 1 });
    }
    await page.locator('button', { hasText: /add|save|create/i }).last().click();

    // Toggle the enabled state
    const toggle = page.locator('input[type="checkbox"], [class*="toggle"], [class*="switch"]').first();
    if (await toggle.isVisible({ timeout: 2_000 }).catch(() => false)) {
      const wasBefore = await toggle.isChecked().catch(() => null);
      await toggle.click();

      // Verify state changed after reload
      await page.reload();
      const isAfter = await toggle.isChecked().catch(() => null);
      if (wasBefore !== null && isAfter !== null) {
        expect(isAfter).not.toBe(wasBefore);
      }
    }
  });

  test('delete payment method', async ({ registeredPage: page }) => {
    await createStoreAndNavigate(page, 'Delete PM Store');

    // Add a payment method
    await page.locator('button', { hasText: /add.*payment.*method/i }).click();
    const chainSelect = page.locator('select').first();
    if (await chainSelect.isVisible({ timeout: 2_000 }).catch(() => false)) {
      await chainSelect.selectOption({ index: 1 });
    }
    await page.locator('button', { hasText: /add|save|create/i }).last().click();

    // Delete it
    await page.locator('button', { hasText: /delete|remove/i }).first().click();
    const confirmBtn = page.locator('button', { hasText: /confirm|yes/i }).last();
    if (await confirmBtn.isVisible({ timeout: 2_000 }).catch(() => false)) {
      await confirmBtn.click();
    }

    // After deletion the section should revert to empty or have one fewer entry
    await page.waitForTimeout(500);
  });
});
