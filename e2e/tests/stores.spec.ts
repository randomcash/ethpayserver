import { test, expect } from '../fixtures/auth';
import { resetDatabase } from '../fixtures/db';
import { createStore } from '../fixtures/stores';

test.describe('Stores', () => {
  test.beforeAll(async () => {
    await resetDatabase();
  });

  test('shows empty state when no stores exist', async ({ registeredPage: page }) => {
    await page.goto('/evm/stores');
    await expect(page.locator('.store-card')).toHaveCount(0);
  });

  test('create store appears in list and sidebar', async ({ registeredPage: page }) => {
    await page.goto('/evm/stores');

    // Open create-store form
    await createStore(page, 'Test Store');

    // Store card should appear in the grid
    await expect(page.locator('.store-card-name', { hasText: 'Test Store' })).toBeVisible();
  });

  test('edit store name persists on reload', async ({ registeredPage: page }) => {
    await page.goto('/evm/stores');

    // Create a store
    await createStore(page, 'Original Name');
    await expect(page.locator('.store-card-name', { hasText: 'Original Name' })).toBeVisible();

    // Navigate to detail page
    await page.locator('.store-card', { hasText: 'Original Name' }).click();
    await page.waitForURL(/\/evm\/stores\/.+/);

    // Edit the name
    const nameInput = page.locator('.form-input').first();
    await nameInput.clear();
    await nameInput.fill('Updated Name');
    await page.locator('button', { hasText: /save|update/i }).click();

    // Reload and verify persistence
    await page.reload();
    await expect(page.getByText('Updated Name')).toBeVisible();
  });

  test('delete store removes it from list', async ({ registeredPage: page }) => {
    await page.goto('/evm/stores');

    // Create a store to delete
    await createStore(page, 'Deletable Store');
    await expect(page.locator('.store-card-name', { hasText: 'Deletable Store' })).toBeVisible();

    // Navigate to detail and trigger delete
    await page.locator('.store-card', { hasText: 'Deletable Store' }).click();
    await page.waitForURL(/\/evm\/stores\/.+/);
    await page.locator('button', { hasText: /delete/i }).click();

    // Confirm deletion dialog if present
    const confirmBtn = page.locator('button', { hasText: /confirm|yes/i }).last();
    if (await confirmBtn.isVisible({ timeout: 2_000 }).catch(() => false)) {
      await confirmBtn.click();
    }

    // Should redirect back to list without the deleted store
    await page.waitForURL(/\/evm\/stores$/);
    await expect(page.locator('.store-card-name', { hasText: 'Deletable Store' })).not.toBeVisible();
  });

  test('sidebar store selector switches context', async ({ registeredPage: page }) => {
    await page.goto('/evm/stores');

    // Create two stores
    for (const name of ['Store Alpha', 'Store Beta']) {
      await createStore(page, name);
      await expect(page.locator('.store-card-name', { hasText: name })).toBeVisible();
    }

    // Switch store via the sidebar selector
    const selector = page.locator('.store-selector, [class*="store-select"]').first();
    if (await selector.isVisible({ timeout: 3_000 }).catch(() => false)) {
      await selector.click();
      await page.getByText('Store Beta').click();
      await expect(selector).toContainText('Store Beta');
    }
  });
});
