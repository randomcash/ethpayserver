import { test, expect } from '../fixtures/auth';
import { resetDatabase } from '../fixtures/db';
import { createStore } from '../fixtures/stores';
import type { Page } from '@playwright/test';

test.describe('Invoices', () => {
  test.beforeAll(async () => {
    await resetDatabase();
  });

  test('create invoice for store', async ({ registeredPage: page }) => {
    await createStore(page, 'Invoice Store');

    // Navigate to invoices page
    await page.goto('/evm/invoices');

    // Open create-invoice modal (button in header or page)
    await page.locator('button', { hasText: /create.*invoice|new.*invoice/i }).click();

    // Fill the create-invoice form
    const storeSelect = page.locator('select').first();
    if (await storeSelect.isVisible({ timeout: 2_000 }).catch(() => false)) {
      await storeSelect.selectOption({ index: 1 });
    }

    const amountInput = page.locator('input[name="amount"], input[placeholder*="amount" i], input[type="number"]').first();
    await amountInput.fill('0.01');

    await page.locator('button', { hasText: /create|submit/i }).last().click();

    // Invoice should appear in the list
    await expect(page.locator('table tbody tr, [class*="invoice-row"], [class*="invoice-card"]').first()).toBeVisible();
  });

  test('invoice appears in list', async ({ registeredPage: page }) => {
    await createStore(page, 'List Invoice Store');
    await page.goto('/evm/invoices');

    // Create an invoice
    await page.locator('button', { hasText: /create.*invoice|new.*invoice/i }).click();
    const storeSelect = page.locator('select').first();
    if (await storeSelect.isVisible({ timeout: 2_000 }).catch(() => false)) {
      await storeSelect.selectOption({ index: 1 });
    }
    const amountInput = page.locator('input[name="amount"], input[placeholder*="amount" i], input[type="number"]').first();
    await amountInput.fill('0.05');
    await page.locator('button', { hasText: /create|submit/i }).last().click();

    // Reload the list and verify the invoice is still there
    await page.goto('/evm/invoices');
    await expect(page.locator('table tbody tr, [class*="invoice-row"], [class*="invoice-card"]').first()).toBeVisible();
  });

  test('invoice detail shows correct data', async ({ registeredPage: page }) => {
    await createStore(page, 'Detail Invoice Store');
    await page.goto('/evm/invoices');

    // Create an invoice
    await page.locator('button', { hasText: /create.*invoice|new.*invoice/i }).click();
    const storeSelect = page.locator('select').first();
    if (await storeSelect.isVisible({ timeout: 2_000 }).catch(() => false)) {
      await storeSelect.selectOption({ index: 1 });
    }
    const amountInput = page.locator('input[name="amount"], input[placeholder*="amount" i], input[type="number"]').first();
    await amountInput.fill('1.23');
    await page.locator('button', { hasText: /create|submit/i }).last().click();

    // Click into the invoice detail
    await page.locator('table tbody tr, [class*="invoice-row"], [class*="invoice-card"]').first().click();
    await page.waitForURL(/\/evm\/invoices\/.+/);

    // Verify detail page shows the amount
    await expect(page.getByText('1.23')).toBeVisible();
  });
});
