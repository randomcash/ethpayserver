import { test as base, expect, type Page, type CDPSession } from '@playwright/test';
import { validateMnemonic } from '@scure/bip39';
import { wordlist } from '@scure/bip39/wordlists/english.js';

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

/**
 * What registration hands the user, and the only things that can recover the
 * account later (RCS-205).
 *
 * A passkey-only account has no email and no wallet address, so `accountId` is
 * the sole identifier it can present at recovery — the phrase alone is not
 * enough. Both are shown once, on the same screen, and never again.
 */
export interface RecoveryCredentials {
  /** The 24 BIP39 words, in order. */
  mnemonic: string[];
  /** Shown only for passkey-only accounts, which have no other handle. */
  accountId: string | null;
}

export async function register(page: Page): Promise<RecoveryCredentials> {
  await page.goto('/register');

  // Select the passkey tab
  await page.locator('.ps-auth-tab', { hasText: /passkey/i }).click();

  // Fill username if a text input is present in the passkey form
  const usernameInput = page.locator('.ps-passkey-form input:not([type="hidden"]):not([type="checkbox"])');
  if (await usernameInput.isVisible({ timeout: 1_000 }).catch(() => false)) {
    // Date.now() alone collides when parallel workers register in the same
    // millisecond. That was harmless while every run started from a reset
    // database; now that this spec runs against shared environments, a
    // collision would surface as a confusing duplicate-account failure.
    const unique = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
    await usernameInput.fill(`e2e_user_${unique}`);
  }

  // Trigger passkey creation — the virtual authenticator handles the prompt
  await page.locator('.ps-passkey-button').click();

  // Registration pauses on the recovery-phrase step before it redirects. Wait
  // for whichever arrives first instead of giving that step a fixed budget: a
  // cold server answers the passkey round trip in more than the 5s an earlier
  // version allowed, and the wait below then timed out against a page still
  // sitting on the recovery screen.
  //
  // Two shapes are handled on purpose (RCS-214). "Skip for Now" is being
  // removed from the default registration flow, because skipping strands the
  // account with no recovery route. Accepting either shape keeps this fixture
  // working on both sides of that change, so the ui-kit default can flip
  // without a synchronised merge.
  const skipButton = page.locator('.ps-button-ghost', { hasText: /skip/i });
  const savedButton = page.locator('.ps-button-primary', { hasText: /written it down/i });
  const settled = /\/(evm)?$/;
  await Promise.race([
    page.waitForURL(settled, { timeout: 30_000 }).catch(() => {}),
    skipButton.waitFor({ state: 'visible', timeout: 30_000 }).catch(() => {}),
    savedButton.waitFor({ state: 'visible', timeout: 30_000 }).catch(() => {}),
  ]);

  // Capture the recovery material BEFORE dismissing the screen. It is displayed
  // exactly once and is unrecoverable afterwards by design, so a test that needs
  // it has this one opportunity. Reading it here rather than in each test also
  // means the selectors live in one place.
  const credentials: RecoveryCredentials = { mnemonic: [], accountId: null };

  // `require_recovery` defaults to true (RCS-214), so reaching this screen is
  // not optional and neither is capturing it. A conditional capture would
  // return an empty phrase on any markup change and let a later recovery test
  // run against nothing - passing silently as coverage, which is the outcome
  // this work exists to avoid. So: throw rather than skip.
  const wordLocator = page.locator('.ps-mnemonic-word');
  await wordLocator.first().waitFor({ state: 'visible', timeout: 10_000 });

  const words = await wordLocator.all();
  for (const [i, word] of words.entries()) {
    // Explicit short timeout: if these inner selectors are renamed in ui-kit
    // while .ps-mnemonic-word survives, fail with a legible selector error
    // rather than hanging until the 30s per-test budget expires.
    const text = (await word.locator('.ps-mnemonic-text').textContent({ timeout: 2_000 }))?.trim();
    if (!text) throw new Error(`recovery phrase word ${i + 1} is empty`);
    credentials.mnemonic.push(text);
  }

  // BIP39 checksum, not a shape check.
  //
  // This replaces an earlier assertion that each word's displayed index matched
  // its position. That check could never fail: RecoverySetup renders the index
  // from `enumerate()`, so it was derived from the same DOM order it was being
  // compared against. A shuffled phrase would have rendered 1..24 against the
  // shuffled words and passed.
  //
  // The checksum is the real check. BIP39 encodes a checksum over the entropy in
  // the final word, so reordering, substituting or dropping a word fails here -
  // at registration, loudly - instead of at recovery, where it is
  // indistinguishable from a merchant typing the wrong phrase.
  //
  // The phrase is deliberately not in the message: it would land in CI logs and
  // the uploaded playwright-report artifact.
  if (!validateMnemonic(credentials.mnemonic.join(' '), wordlist)) {
    throw new Error(
      `registration produced a phrase failing BIP39 validation ` +
        `(${credentials.mnemonic.length} words); value withheld from logs`,
    );
  }

  const accountId = page.locator('.ps-recovery-account-id-value');
  if (await accountId.isVisible().catch(() => false)) {
    credentials.accountId =
      ((await accountId.textContent({ timeout: 2_000 })) ?? '').trim() || null;
  }

  if (await skipButton.isVisible().catch(() => false)) {
    await skipButton.click();
  } else if (await savedButton.isVisible().catch(() => false)) {
    // Confirm path: acknowledge the phrase, tick the attestation, complete.
    // There is no word re-entry, so the fixture does not need to type it back.
    await savedButton.click();
    await page.locator('.ps-checkbox').check();
    await page.locator('.ps-button-primary', { hasText: /complete setup/i }).click();
  }

  // Wait for redirect to dashboard
  await page.waitForURL(settled, { timeout: 15_000 });

  return credentials;
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
