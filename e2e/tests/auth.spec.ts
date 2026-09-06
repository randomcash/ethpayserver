import { test, expect, register, login } from '../fixtures/auth';

// Opt-out only. This used to default to ON whenever E2E_REMOTE was set, for a
// reason that does not hold up: the README said the virtual authenticator's RP
// ID ("localhost") could not match a remote domain, but
// `WebAuthn.addVirtualAuthenticator` has no RP ID parameter at all. It takes
// protocol, transport, hasResidentKey, hasUserVerification and isUserVerified.
// The RP ID comes from the page's origin when navigator.credentials.create()
// runs, so an authenticator on testnet.random.cash gets that RP ID by
// construction. The deployed server agrees - it logs
// `rp_id=testnet.random.cash rp_origin=https://testnet.random.cash` at startup.
//
// The genuine blocker was below: this spec reset the database.
const SKIP_AUTH = process.env.E2E_SKIP_AUTH === 'true';

test.describe('Authentication', () => {
  test.skip(() => SKIP_AUTH, 'Skipped explicitly via E2E_SKIP_AUTH=true');

  // Deliberately no resetDatabase(). Every test here creates its own account
  // through register(), which mints a unique username, so none of them needs an
  // empty database - the reset was hygiene, and hygiene that made the whole
  // spec unusable against any shared environment. Deleting rows from a live
  // testnet to run a login test is not a trade worth making.
  //
  // Six other specs (invoices, stores, payment-methods, ui-interactions,
  // webhooks and formerly this one) still reset and remain local-only.

  test('register new account with passkey', async ({ withAuthenticator: page }) => {
    await register(page);

    await expect(page).toHaveURL(/\/(evm)?$/);
    await expect(page.locator('.ps-auth-page')).not.toBeVisible();
  });

  test('registration surfaces the recovery phrase and account id', async ({
    withAuthenticator: page,
  }) => {
    const credentials = await register(page);

    // Assert on the count, never on the array: a failing `toHaveLength` prints
    // the received value, which would put real recovery material into CI logs
    // and the uploaded playwright-report artifact. register_page.rs withholds
    // Debug from mnemonic_words for the same reason (RCS-193).
    //
    // Word validity is not re-checked here - the fixture already validates the
    // BIP39 checksum and refuses to return a phrase that fails, which catches
    // reordering and substitution that a per-word regex cannot.
    expect(credentials.mnemonic.length, 'phrase should be 24 words').toBe(24);

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
