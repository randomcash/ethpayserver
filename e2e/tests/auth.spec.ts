import { test, expect, register, login } from '../fixtures/auth';
import { resetDatabase } from '../fixtures/db';

test.describe('Authentication', () => {
  test.beforeAll(async () => {
    await resetDatabase();
  });

  test('register new account with passkey', async ({ withAuthenticator: page }) => {
    await register(page);

    await expect(page).toHaveURL(/\/(evm)?$/);
    await expect(page.locator('.ps-auth-page')).not.toBeVisible();
  });

  test('login with existing passkey', async ({ withAuthenticator: page }) => {
    // Register first so the virtual authenticator holds a credential
    await register(page);

    // Clear the session but keep the virtual authenticator state
    await page.context().clearCookies();
    await page.evaluate(() => localStorage.clear());

    // Login re-uses the resident credential from the authenticator
    await login(page);
    await expect(page).toHaveURL(/\/(evm)?$/);
  });

  test('session persists across page reload', async ({ withAuthenticator: page }) => {
    await register(page);
    await expect(page).toHaveURL(/\/(evm)?$/);

    await page.reload();

    await expect(page).toHaveURL(/\/(evm)?$/);
    await expect(page.locator('.ps-auth-page')).not.toBeVisible();
  });

  test('unauthenticated users are redirected to login', async ({ page }) => {
    await page.goto('/evm/stores');
    await expect(page).toHaveURL(/\/login/);
  });
});
