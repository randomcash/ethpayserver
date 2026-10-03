import { test, expect, register } from '../fixtures/auth';
import { promoteToServerAdmin } from '../fixtures/db';

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
    await page.goto('/evm/settings');
    // Wait for a tab every account has, so absence of the admin one is not just
    // "the page has not rendered yet".
    await expect(page.locator('.settings-tab', { hasText: /account/i }).first()).toBeVisible();
    // The tab is gated on the role fetch, which can resolve after the first
    // tabs paint; let the network settle before asserting absence.
    await page.waitForLoadState('networkidle');
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
});
