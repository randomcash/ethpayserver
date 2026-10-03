import { test, expect, register } from '../fixtures/auth';

/**
 * The Server Admin tab changes who is an admin, so an ordinary account must
 * never be offered it - not hidden by styling, absent from the page.
 */
test.describe('Server Admin tab visibility', () => {
  test('a non-admin account does not see the Server Admin tab', async ({
    withAuthenticator: page,
  }) => {
    await register(page);
    await page.goto('/evm/settings');
    // The tabs are rendered once the account's role has loaded; wait for a
    // tab every account has, so absence of the admin one is not just "not yet".
    await expect(page.locator('.settings-tab', { hasText: /account/i }).first()).toBeVisible();
    await expect(page.locator('.settings-tab', { hasText: /server admin/i })).toHaveCount(0);
    await expect(page.locator('.admin-users-table')).toHaveCount(0);
  });
});
