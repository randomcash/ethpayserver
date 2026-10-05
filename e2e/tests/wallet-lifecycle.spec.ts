/**
 * The wallet sibling of account-lifecycle.spec.ts.
 *
 * Wallet authentication can create accounts and had no end-to-end coverage of
 * any kind: auth.spec.ts mentions a wallet only in a comment explaining that a
 * passkey account does not have one. So an entire authentication method shipped
 * untested. No chain and no funds are involved — the signature is EIP-191 over
 * a server-issued challenge — so this gates every push like the rest.
 */

import { test, expect } from '../fixtures/auth';
import { logout } from '../fixtures/auth';
import {
  installMockWallet,
  registerWithWallet,
  loginWithWallet,
  USER_REJECTED,
} from '../fixtures/wallet-provider';
import { createStoreReadyForInvoices } from '../fixtures/payment-methods';
import { createInvoice } from '../fixtures/invoices';

test.describe('Account lifecycle (wallet)', () => {
  test('an account can be created by signing, used, returned to, and destroyed', async ({
    page,
  }) => {
    const wallet = await installMockWallet(page);

    await registerWithWallet(page, wallet);

    // A wallet account carries its address, and unlike a passkey-only account
    // it does not need the account-id fallback to identify itself.
    //
    // Asserted through the settings page rather than GET /api/auth/me: the
    // client authenticates with a Bearer token it holds in the page, so
    // `page.request` carries no credentials and that endpoint answers 401 for
    // any state of the world. An assertion that cannot fail is worse than none.
    await page.goto('/evm/settings');
    await expect(
      page.locator('code', { hasText: new RegExp(wallet.address, 'i') }),
      'settings should show the address that signed as the primary wallet',
    ).toBeVisible({ timeout: 15_000 });

    // Give it real state, so deletion has something to remove rather than
    // succeeding trivially against an empty account.
    const store = `wallet-store-${Date.now().toString(36)}`;
    await createStoreReadyForInvoices(page, store);
    await createInvoice(page, '0.001');

    const afterRegister = await wallet.signatureCount();
    expect(afterRegister, 'registration should have signed exactly once').toBe(1);

    await logout(page);
    await loginWithWallet(page);

    // The assertion that matters: logging back in signs again. If the client
    // replayed a stored signature this count would not move, and a replayable
    // login is the bug this spec exists to catch.
    expect(
      await wallet.signatureCount(),
      'logging back in should sign a fresh challenge, not replay the first',
    ).toBe(afterRegister + 1);

    // ---- destroy it ----------------------------------------------------
    await page.goto('/evm/settings');
    await page.locator('.settings-tab', { hasText: /account/i }).click();

    const danger = page.locator('.ps-card-danger');
    await expect(danger, 'the Danger Zone should be on the Account tab').toBeVisible();
    await danger.locator('button', { hasText: /delete account/i }).click();

    const confirm = page.locator('.form-group', { hasText: /to confirm/i });
    await expect(confirm).toBeVisible();

    // Read the handle off the label rather than assuming it. The server
    // compares against the email where there is one and the account id
    // otherwise, so a wallet account is confirmed by its id — not, as the
    // wallet tab might lead you to expect, by its address.
    const handle = ((await confirm.locator('code').textContent()) ?? '').trim();
    expect(handle, 'the confirmation should name the handle to type').not.toBe('');

    const destroy = confirm.locator('button', { hasText: /permanently delete/i });
    await expect(destroy, 'must not be armed before the handle is typed').toBeDisabled();
    await confirm.locator('input[type="text"]').fill(handle);
    await expect(destroy).toBeEnabled();
    await destroy.click();

    await page.waitForURL(/\/login/, { timeout: 15_000 });

    // Gone, not merely logged out: the wallet that created the account must no
    // longer open anything. The key still exists and still signs — that is the
    // whole point of the assertion.
    await loginWithWallet(page).catch(() => {});
    await expect(
      page,
      'a deleted account must not be reachable by its surviving wallet',
    ).toHaveURL(/\/login/, { timeout: 15_000 });
  });

  test('a declined signature is reported, not swallowed', async ({ page }) => {
    const wallet = await installMockWallet(page);
    await wallet.rejectNextSignature();

    await page.goto('/register');
    await page.locator('.ps-auth-tab', { hasText: /wallet/i }).click();
    await page.locator('.ps-wallet-button').click();

    // Declining is the single most common thing a real user does at this
    // prompt. It must surface as readable text — not a hang, and not a silent
    // return to an idle button that leaves the user with no idea what happened.
    const error = page.locator('.ps-auth-error, .ps-wallet-error').first();
    await expect(error, 'declining the signature should say so').toBeVisible({
      timeout: 15_000,
    });
    await expect(error).not.toBeEmpty();

    // And it must not have logged anyone in behind the rejection. Checked by
    // navigation, not by GET /api/auth/me: that endpoint 401s without a Bearer
    // token no matter what, so asserting on it would pass even if the decline
    // had created and signed in an account.
    await page.goto('/evm/settings');
    await expect(
      page,
      'a declined signature must not leave the browser logged in',
    ).toHaveURL(/\/login/, { timeout: 15_000 });

    void USER_REJECTED;
  });

  test('a second registration with the same address is refused', async ({ page }) => {
    const wallet = await installMockWallet(page);
    await registerWithWallet(page, wallet);
    await logout(page);

    // Same provider, same address, second registration. This must be refused
    // rather than quietly creating a duplicate account: addresses are how
    // wallet accounts are identified, so two accounts sharing one make the
    // login target ambiguous.
    await page.goto('/register');
    await page.locator('.ps-auth-tab', { hasText: /wallet/i }).click();
    await page.locator('.ps-wallet-button').click();

    const error = page.locator('.ps-auth-error, .ps-wallet-error').first();
    await expect(
      error,
      'registering an address twice should be refused with a reason',
    ).toBeVisible({ timeout: 15_000 });
  });
});
