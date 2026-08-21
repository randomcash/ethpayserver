import { test, expect } from '../fixtures/auth';
import { resetDatabase } from '../fixtures/db';
import { createStore, createStoreAndOpen, selectStore } from '../fixtures/stores';

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

    // Scoped to the General tab: a bare `.form-input` resolves to the Create
    // Invoice modal's amount field, which is mounted in the layout and hidden.
    const general = page.locator('.store-tab-general');
    const nameInput = general.locator('.form-input').first();
    await nameInput.fill('Updated Name');

    // Wait for the save to land before reloading — clicking and reloading
    // immediately races the request, and the reload wins often enough that the
    // page comes back showing the original name.
    await Promise.all([
      // Deliberately not filtering on `response.ok()`: a rejected PUT would
      // then match nothing and surface as a bare "timeout waiting for
      // response" instead of the status the server actually returned.
      page.waitForResponse(
        (response) =>
          /\/api\/stores\/[^/]+$/.test(new URL(response.url()).pathname) &&
          response.request().method() === 'PUT',
      ).then(async (response) => {
        expect(response.ok(), `PUT ${new URL(response.url()).pathname} → ${response.status()}`).toBe(
          true,
        );
        return response;
      }),
      general.locator('.form-actions .btn-primary').click(),
    ]);

    // Reload and verify persistence
    await page.reload();
    await expect(page.locator('.store-tab-general .form-input').first()).toHaveValue('Updated Name');
  });

  test('delete store removes it from list', async ({ registeredPage: page }) => {
    await page.goto('/evm/stores');

    // Create a store to delete
    await createStore(page, 'Deletable Store');
    await expect(page.locator('.store-card-name', { hasText: 'Deletable Store' })).toBeVisible();

    // Navigate to detail and trigger delete
    await page.locator('.store-card', { hasText: 'Deletable Store' }).click();
    await page.waitForURL(/\/evm\/stores\/.+/);

    // Deletion is guarded by a native `window.confirm`, which Playwright
    // auto-dismisses — so the old spec's hunt for a confirm *button* left the
    // store undeleted and waited out its timeout on a navigation that could
    // never happen.
    page.once('dialog', (dialog) => dialog.accept());
    await page.locator('.store-tab-general .btn-danger').click();

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

    // Switch store via the sidebar selector. `getByText('Store Beta')` matched
    // both the dropdown entry and the store card, so this goes through the
    // scoped helper.
    // `.store-selector-name`, not `.store-selector`: the latter also wraps
    // `.store-dropdown`, which renders an entry for *every* store, so
    // `toContainText('Store Beta')` there passes whichever store is selected.
    // `.store-selector-name` holds only the current selection.
    await selectStore(page, 'Store Beta');
    await expect(page.locator('.store-selector-name')).toHaveText('Store Beta');

    await selectStore(page, 'Store Alpha');
    await expect(page.locator('.store-selector-name')).toHaveText('Store Alpha');
  });
});
