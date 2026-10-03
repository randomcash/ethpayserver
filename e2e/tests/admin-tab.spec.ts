import { test, expect, register } from '../fixtures/auth';
import { promoteToServerAdmin, createUserWithApiKey, readUserAccess } from '../fixtures/db';

/**
 * The Server Admin tab changes who is an admin, so an ordinary account must
 * never be offered it - not hidden by styling, absent from the page.
 *
 * Both cases register the same way and differ only in the stored role, so the
 * admin case is the positive control: it proves the selectors below match the
 * real tab and table, which is what gives the absence in the other case meaning.
 */
test.describe('Server Admin tab visibility', () => {
  test('a non-admin account does not see the Server Admin tab', async ({
    withAuthenticator: page,
  }) => {
    await register(page);
    // The tab is gated on the role fetch (/api/auth/me), which can resolve after
    // the first tabs paint. Wait for that response itself, not for the network
    // to go idle, so absence is measured after the role is known.
    const roleLoaded = page.waitForResponse(
      (r) => r.url().includes('/api/auth/me') && r.ok(),
    );
    await page.goto('/evm/settings');
    await roleLoaded;
    // Also wait for a tab every account has, so absence of the admin one is not
    // just "the page has not rendered yet".
    await expect(page.locator('.settings-tab', { hasText: /account/i }).first()).toBeVisible();
    await expect(page.locator('.settings-tab-admin')).toHaveCount(0);
    await expect(page.locator('.settings-tab', { hasText: /server admin/i })).toHaveCount(0);
    await expect(page.locator('.admin-users-table')).toHaveCount(0);
  });

  test('a server admin account does see the tab and the user table', async ({
    withAuthenticator: page,
  }) => {
    const { accountId } = await register(page);
    expect(accountId, 'a passkey-only account must surface its id').toBeTruthy();
    await promoteToServerAdmin(accountId!);

    await page.goto('/evm/settings');
    const tab = page.locator('.settings-tab-admin', { hasText: /server admin/i });
    await expect(tab).toBeVisible();
    await tab.click();
    await expect(page.locator('.admin-users-table')).toBeVisible();
  });

  test('an admin changes another account\'s role and locks it, and the server keeps both', async ({
    withAuthenticator: page,
  }) => {
    const { accountId } = await register(page);
    await promoteToServerAdmin(accountId!);
    const other = await createUserWithApiKey('user');

    await page.goto('/evm/settings');
    await page.locator('.settings-tab-admin', { hasText: /server admin/i }).click();
    const row = page.locator('.admin-users-table tr.user-row', { hasText: other.email });
    await expect(row).toBeVisible();

    const roleSaved = page.waitForResponse(
      (r) => /\/api\/admin\/users\/.+\/role/.test(r.url()) && r.request().method() !== 'GET',
    );
    await row.locator('select').selectOption('server_admin');
    expect((await roleSaved).ok()).toBe(true);
    // The stored role, not the dropdown, is what decides who is an admin.
    await expect.poll(async () => (await readUserAccess(other.userId)).role).toBe('server_admin');

    const lockSaved = page.waitForResponse(
      (r) => /\/api\/admin\/users\/.+\/lock/.test(r.url()) && r.ok(),
    );
    await row.getByRole('button', { name: 'Lock' }).click();
    await lockSaved;
    await expect.poll(async () => (await readUserAccess(other.userId)).locked).toBe(true);
    // The reload after the change keeps the admin on the same page and shows it.
    await expect(row.getByRole('button', { name: 'Unlock' })).toBeVisible();
  });
});
