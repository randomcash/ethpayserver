import { test as base, expect, type Page, type CDPSession } from '@playwright/test';

export async function setupVirtualAuthenticator(
  page: Page,
): Promise<{ cdpSession: CDPSession; authenticatorId: string }> {
  const client = await page.context().newCDPSession(page);
  await client.send('WebAuthn.enable');
  const { authenticatorId } = await client.send('WebAuthn.addVirtualAuthenticator', {
    options: {
      protocol: 'ctap2',
      transport: 'internal',
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
    },
  });
  return { cdpSession: client, authenticatorId };
}

export async function register(page: Page): Promise<void> {
  await page.goto('/register');

  // Select the passkey tab
  await page.locator('.ps-auth-tab', { hasText: /passkey/i }).click();

  // Fill username if a text input is present in the passkey form
  const usernameInput = page.locator('.ps-passkey-form input:not([type="hidden"]):not([type="checkbox"])');
  if (await usernameInput.isVisible({ timeout: 1_000 }).catch(() => false)) {
    await usernameInput.fill(`e2e_user_${Date.now()}`);
  }

  // Trigger passkey creation — the virtual authenticator handles the prompt
  await page.locator('.ps-passkey-button').click();

  // Registration pauses on the recovery-phrase step before it redirects. Wait
  // for whichever of the two arrives first instead of giving that step a fixed
  // budget: a cold server answers the passkey round trip in more than the 5s
  // the previous version allowed, and the wait below then timed out against a
  // page still sitting on the recovery screen.
  const skipButton = page.locator('.ps-button-ghost', { hasText: /skip/i });
  const settled = /\/(evm)?$/;
  await Promise.race([
    page.waitForURL(settled, { timeout: 30_000 }).catch(() => {}),
    skipButton.waitFor({ state: 'visible', timeout: 30_000 }).catch(() => {}),
  ]);
  if (await skipButton.isVisible().catch(() => false)) {
    await skipButton.click();
  }

  // Wait for redirect to dashboard
  await page.waitForURL(settled, { timeout: 15_000 });
}

export async function login(page: Page): Promise<void> {
  await page.goto('/login');
  await page.locator('.ps-auth-tab', { hasText: /passkey/i }).click();
  await page.locator('.ps-passkey-button').click();
  await page.waitForURL(/\/(evm)?$/, { timeout: 15_000 });
}

export interface AuthFixtures {
  withAuthenticator: Page;
  registeredPage: Page;
}

export const test = base.extend<AuthFixtures>({
  withAuthenticator: async ({ page }, use) => {
    await setupVirtualAuthenticator(page);
    await use(page);
  },

  registeredPage: async ({ page }, use) => {
    await setupVirtualAuthenticator(page);
    await register(page);
    await use(page);
  },
});

export { expect };
