import { test, expect, register, login } from '../fixtures/auth';
import { resetDatabase } from '../fixtures/db';

const SKIP_AUTH = process.env.E2E_SKIP_AUTH
  ? process.env.E2E_SKIP_AUTH === 'true'
  : process.env.E2E_REMOTE === 'true';

test.describe('Authentication', () => {
  test.skip(() => SKIP_AUTH, 'Skipped: passkey origin mismatch in remote mode (E2E_SKIP_AUTH)');
  test.beforeAll(async () => {
    await resetDatabase();
  });

  test('register new account with passkey', async ({ withAuthenticator: page }) => {
    await register(page);

    await expect(page).toHaveURL(/\/(evm)?$/);
    await expect(page.locator('.ps-auth-page')).not.toBeVisible();
  });

  test('registration surfaces the recovery phrase and account id', async ({
    withAuthenticator: page,
  }) => {
    const credentials = await register(page);

    // 24 words, and the fixture already asserted they are in displayed order.
    // A phrase that is short, empty or reordered still looks plausible on
    // screen; it only fails much later, at recovery, as an error identical to
    // "wrong phrase" (RCS-205).
    expect(credentials.mnemonic).toHaveLength(24);
    for (const [i, word] of credentials.mnemonic.entries()) {
      expect(word, `word ${i + 1} should be a BIP39 word`).toMatch(/^[a-z]+$/);
    }

    // A passkey-only account has no email and no wallet, so the account id is
    // the ONLY identifier it can present at recovery. Losing it means the phrase
    // alone cannot recover the account, so its presence is load-bearing rather
    // than cosmetic.
    expect(credentials.accountId, 'passkey-only accounts must be shown an account id').toBeTruthy();
    expect(credentials.accountId).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i,
    );
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
