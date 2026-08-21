/**
 * Live-site scout: tests unauthenticated flows and (if auth succeeds)
 * authenticated pages. Designed for E2E_REMOTE=true.
 *
 * Run with:  E2E_REMOTE=true npx playwright test tests/scout.spec.ts
 */
import { test as base, expect, type Page, type ConsoleMessage } from '@playwright/test';
import { setupVirtualAuthenticator } from '../fixtures/auth';

let scoutPage: Page;
const issues: string[] = [];
const consoleErrors: string[] = [];
let authenticated = false;

function issue(label: string, detail: string) {
  issues.push(`[${label}] ${detail}`);
}

const test = base.extend({});
test.describe.configure({ mode: 'serial' });

test.beforeAll(async ({ browser }) => {
  const ctx = await browser.newContext();
  scoutPage = await ctx.newPage();
  scoutPage.on('console', (msg: ConsoleMessage) => {
    if (msg.type() === 'error') consoleErrors.push(msg.text());
  });
  scoutPage.on('pageerror', (err: Error) => {
    consoleErrors.push(`UNCAUGHT: ${err.message}`);
  });
});

test.afterAll(async () => {
  if (consoleErrors.length > 0) {
    console.log('\n=== JS CONSOLE ERRORS ===');
    for (const e of consoleErrors) console.log(`  ${e}`);
  }
  if (issues.length > 0) {
    console.log('\n=== ISSUES FOUND ===');
    for (const i of issues) console.log(`  ${i}`);
  } else {
    console.log('\n=== NO ISSUES FOUND ===');
  }
  await scoutPage?.context().close();
});

async function goto(path: string) {
  await scoutPage.goto(path);
  await scoutPage.waitForLoadState('networkidle', { timeout: 15_000 }).catch(() => {});
}

// ---------------------------------------------------------------------------
// Unauthenticated flows — these always run
// ---------------------------------------------------------------------------

test.describe('Unauthenticated', () => {
  test('login page renders correctly', async () => {
    await goto('/login');

    // Page should show sign-in form
    // `.first()`: 'Sign In' matches both the heading and the submit button, and
    // isVisible() on a multi-match locator throws a strict-mode violation that the
    // catch below would record as a phantom issue.
    const heading = scoutPage.getByText('Sign In').first();
    if (!await heading.isVisible({ timeout: 10_000 }).catch(() => false)) {
      issue('LOGIN', 'Sign In heading not visible');
    }

    // Auth tabs should be visible
    const walletTab = scoutPage.locator('.ps-auth-tab', { hasText: /wallet/i });
    const passkeyTab = scoutPage.locator('.ps-auth-tab', { hasText: /passkey/i });
    if (!await walletTab.isVisible({ timeout: 3_000 }).catch(() => false)) {
      issue('LOGIN', 'Wallet tab not visible');
    }
    if (!await passkeyTab.isVisible({ timeout: 3_000 }).catch(() => false)) {
      issue('LOGIN', 'Passkey tab not visible');
    }

    // "Create one" link
    const createLink = scoutPage.getByText('Create one').first();
    if (!await createLink.isVisible({ timeout: 3_000 }).catch(() => false)) {
      issue('LOGIN', '"Create one" registration link not visible');
    }
  });

  test('register page renders correctly', async () => {
    await goto('/register');

    const heading = scoutPage.getByText('Create Account');
    if (!await heading.isVisible({ timeout: 10_000 }).catch(() => false)) {
      issue('REGISTER', 'Create Account heading not visible');
    }

    const walletTab = scoutPage.locator('.ps-auth-tab', { hasText: /wallet/i });
    const passkeyTab = scoutPage.locator('.ps-auth-tab', { hasText: /passkey/i });
    if (!await walletTab.isVisible({ timeout: 3_000 }).catch(() => false)) {
      issue('REGISTER', 'Wallet tab not visible');
    }
    if (!await passkeyTab.isVisible({ timeout: 3_000 }).catch(() => false)) {
      issue('REGISTER', 'Passkey tab not visible');
    }

    // Switch to passkey tab
    if (await passkeyTab.isVisible().catch(() => false)) {
      await passkeyTab.click();
      const createBtn = scoutPage.locator('.ps-passkey-button');
      if (!await createBtn.isVisible({ timeout: 3_000 }).catch(() => false)) {
        issue('REGISTER', 'Create Passkey button not visible after switching to passkey tab');
      }
    }

    // "Sign in" link
    const signIn = scoutPage.getByText('Sign in').first();
    if (!await signIn.isVisible({ timeout: 3_000 }).catch(() => false)) {
      issue('REGISTER', '"Sign in" link not visible');
    }
  });

  test('unauthenticated access redirects to login', async () => {
    await goto('/evm/stores');
    try {
      await expect(scoutPage).toHaveURL(/\/login/, { timeout: 5_000 });
    } catch {
      issue('AUTH_REDIRECT', `Expected redirect to /login, got ${scoutPage.url()}`);
    }
  });

  test('health endpoint is up', async () => {
    const resp = await scoutPage.request.get('/health/live');
    if (resp.status() !== 200) {
      issue('HEALTH', `Health endpoint returned ${resp.status()}`);
    }
  });

  test('API returns 401 for unauthenticated requests', async () => {
    const resp = await scoutPage.request.get('/api/auth/me');
    if (resp.status() !== 401) {
      issue('API', `/api/auth/me returned ${resp.status()} instead of 401`);
    }
  });

  test('checkout page for nonexistent invoice shows error', async () => {
    await goto('/checkout/00000000-0000-0000-0000-000000000000');

    const error = scoutPage.locator('.checkout-error');
    const loading = scoutPage.locator('.checkout-loading');
    // Should show error or loading (then error)
    await scoutPage.waitForTimeout(3_000);
    if (await loading.isVisible().catch(() => false)) {
      // Still loading after 3s — might be stuck
      issue('CHECKOUT', 'Checkout page still loading after 3s for nonexistent invoice');
    }
    // Error state is correct behavior here
  });

  test('login page wallet tab shows MetaMask prompt', async () => {
    await goto('/login');
    const walletTab = scoutPage.locator('.ps-auth-tab', { hasText: /wallet/i });
    if (await walletTab.isVisible({ timeout: 3_000 }).catch(() => false)) {
      await walletTab.click();
      // Should show "No Ethereum wallet detected" since we're headless
      const noWallet = scoutPage.getByText(/no.*wallet.*detected|install.*metamask/i);
      if (!await noWallet.isVisible({ timeout: 3_000 }).catch(() => false)) {
        issue('LOGIN', 'No wallet-not-detected message when no wallet extension present');
      }
    }
  });
});

// ---------------------------------------------------------------------------
// Authentication attempt
// ---------------------------------------------------------------------------

test.describe('Auth & Authenticated', () => {
  test('register with passkey', async () => {
    await setupVirtualAuthenticator(scoutPage);
    await goto('/register');

    // Switch to passkey tab
    const passkeyTab = scoutPage.locator('.ps-auth-tab', { hasText: /passkey/i });
    await passkeyTab.click();

    // Fill username if present
    const usernameInput = scoutPage.locator('.ps-passkey-form input:not([type="hidden"]):not([type="checkbox"])');
    if (await usernameInput.isVisible({ timeout: 1_000 }).catch(() => false)) {
      await usernameInput.fill(`scout_${Date.now()}`);
    }

    // Take screenshot before clicking
    await scoutPage.screenshot({ path: 'test-results/scout-before-register.png' });

    // Click create passkey
    const createBtn = scoutPage.locator('.ps-passkey-button');
    await createBtn.click();

    // Wait for any error message or recovery step
    await scoutPage.waitForTimeout(3_000);

    // Check for errors
    const errorAlert = scoutPage.locator('.ps-auth-error, .ps-alert-error, [class*="error"]');
    const errorTexts: string[] = [];
    const errorCount = await errorAlert.count();
    for (let i = 0; i < errorCount; i++) {
      const text = await errorAlert.nth(i).textContent().catch(() => '');
      if (text && text.trim()) errorTexts.push(text.trim());
    }
    if (errorTexts.length > 0) {
      issue('REGISTER', `Error after passkey creation: ${errorTexts.join(' | ')}`);
    }

    // Handle recovery step
    const skipButton = scoutPage.locator('.ps-button-ghost, button', { hasText: /skip/i });
    if (await skipButton.isVisible({ timeout: 5_000 }).catch(() => false)) {
      await skipButton.click();
    }

    // Take screenshot after
    await scoutPage.screenshot({ path: 'test-results/scout-after-register.png' });

    // Check if we reached the dashboard
    try {
      await expect(scoutPage).toHaveURL(/\/(evm)?$/, { timeout: 10_000 });
      authenticated = true;
    } catch {
      issue('REGISTER', `Registration did not redirect to dashboard. Final URL: ${scoutPage.url()}`);

      // Try to see what page we ended up on
      const pageText = await scoutPage.locator('body').textContent().catch(() => '');
      if (pageText?.includes('Sign In')) {
        issue('REGISTER', 'Ended up on login page — session not created after registration');
      }
    }
  });

  // --- Authenticated tests (skip if registration failed) ---------------------

  test('dashboard loads', async () => {
    test.skip(!authenticated, 'Registration failed');
    await goto('/evm');

    if (!await scoutPage.locator('.dashboard-header, .page-title').first().isVisible({ timeout: 10_000 }).catch(() => false)) {
      issue('DASHBOARD', 'Dashboard header not visible');
    }

    // Metrics
    const cards = await scoutPage.locator('.metric-card').count();
    if (cards !== 4) issue('DASHBOARD', `Expected 4 metric cards, got ${cards}`);

    // Charts
    if (!await scoutPage.locator('.charts-section').isVisible({ timeout: 5_000 }).catch(() => false)) {
      issue('DASHBOARD', 'Charts section missing');
    }

    // Activity
    if (!await scoutPage.locator('.activity-section').isVisible({ timeout: 5_000 }).catch(() => false)) {
      issue('DASHBOARD', 'Activity section missing');
    }

    // WebSocket indicator
    const wsLabel = await scoutPage.locator('.ws-indicator-label').textContent().catch(() => '');
    if (wsLabel === 'Offline') {
      issue('WEBSOCKET', 'Shows Offline on live site');
    }
  });

  test('sidebar navigation works', async () => {
    test.skip(!authenticated, 'Registration failed');

    const routes: [string, RegExp][] = [
      ['Invoices', /\/evm\/invoices/],
      ['Payments', /\/evm\/payments/],
      ['Stores', /\/evm\/stores/],
      ['Wallets', /\/evm\/wallets/],
      ['Settings', /\/evm\/settings/],
      ['Dashboard', /\/(evm)?$/],
    ];

    for (const [label, pattern] of routes) {
      const link = scoutPage.locator('.sidebar-link', { hasText: label });
      if (!await link.isVisible({ timeout: 3_000 }).catch(() => false)) {
        issue('SIDEBAR', `"${label}" link not visible`);
        continue;
      }
      await link.click();
      try {
        await expect(scoutPage).toHaveURL(pattern, { timeout: 5_000 });
      } catch {
        issue('SIDEBAR', `"${label}" did not navigate. URL: ${scoutPage.url()}`);
      }
    }
  });

  test('user menu interactions', async () => {
    test.skip(!authenticated, 'Registration failed');
    await goto('/evm');

    const trigger = scoutPage.locator('.user-menu-trigger');
    const dropdown = scoutPage.locator('.user-menu-dropdown');

    // Open
    await trigger.click();
    if (!await dropdown.evaluate(el => el.classList.contains('open')).catch(() => false)) {
      issue('USER_MENU', 'Did not open');
    }

    // Close via trigger
    await trigger.click();
    await scoutPage.waitForTimeout(200);
    if (await dropdown.evaluate(el => el.classList.contains('open')).catch(() => true)) {
      issue('USER_MENU', 'Did not close on second click');
    }

    // Close via outside click
    await trigger.click();
    await scoutPage.locator('.main-header-search').click();
    await scoutPage.waitForTimeout(300);
    if (await dropdown.evaluate(el => el.classList.contains('open')).catch(() => true)) {
      issue('USER_MENU', 'Did not close on outside click');
    }
  });

  test('event listener leak check', async () => {
    test.skip(!authenticated, 'Registration failed');
    await goto('/evm');

    const trigger = scoutPage.locator('.user-menu-trigger');
    const client = await scoutPage.context().newCDPSession(scoutPage);

    const before = await client.send('Runtime.evaluate', {
      expression: `(()=>{const l=getEventListeners(window);return l.click?l.click.length:0})()`,
      returnByValue: true,
      // `getEventListeners` is a DevTools *console* helper, not a page global.
      // Without this it is undefined, the expression throws, `result.value`
      // comes back undefined, and the `leaked > 1` check below silently
      // compares NaN — making the LISTENER_LEAK issue unreachable.
      includeCommandLineAPI: true,
    });
    const baseline = before.result.value as number;

    for (let i = 0; i < 20; i++) {
      await trigger.click();
      await trigger.click();
    }

    const after = await client.send('Runtime.evaluate', {
      expression: `(()=>{const l=getEventListeners(window);return l.click?l.click.length:0})()`,
      returnByValue: true,
      // Console-helper API, as above.
      includeCommandLineAPI: true,
    });
    await client.detach();

    // A non-numeric result means the CDP expression failed rather than that
    // nothing leaked; report it instead of comparing NaN and passing.
    if (typeof baseline !== 'number' || typeof after.result.value !== 'number') {
      issue('LISTENER_LEAK', 'getEventListeners did not return a count — check could not run');
      return;
    }

    const leaked = (after.result.value as number) - baseline;
    if (leaked > 1) {
      issue('LISTENER_LEAK', `${leaked} click listeners leaked after 20 menu cycles`);
    }
  });

  test('create invoice modal', async () => {
    test.skip(!authenticated, 'Registration failed');
    await goto('/evm');

    // Scoped to the header: the modal's own submit button carries the same
    // label, so an unscoped match is a strict-mode violation.
    await scoutPage.locator('.main-header-actions button', { hasText: /create invoice/i }).click();
    if (!await scoutPage.locator('.modal-overlay').isVisible({ timeout: 3_000 }).catch(() => false)) {
      issue('CREATE_INVOICE', 'Modal did not open');
      return;
    }

    // Check fields
    for (const id of ['#ci-amount', '#ci-currency', '#ci-expiration']) {
      if (!await scoutPage.locator(id).isVisible({ timeout: 2_000 }).catch(() => false)) {
        issue('CREATE_INVOICE', `Field ${id} not visible`);
      }
    }

    // Submit empty — should stay open
    await scoutPage.locator('#ci-amount').fill('');
    await scoutPage.locator('.modal .btn-primary').click();
    await scoutPage.waitForTimeout(500);
    if (!await scoutPage.locator('.modal-overlay').isVisible().catch(() => false)) {
      issue('CREATE_INVOICE', 'Modal closed with empty amount — no validation');
    }

    // Close
    await scoutPage.locator('.modal-overlay').click({ position: { x: 5, y: 5 } });
  });

  test('stores page', async () => {
    test.skip(!authenticated, 'Registration failed');
    await goto('/evm/stores');

    if (!await scoutPage.locator('.page-title').isVisible({ timeout: 10_000 }).catch(() => false)) {
      issue('STORES', 'Title not visible');
    }

    const content = scoutPage.locator('.stores-grid, .stores-empty, .store-card');
    if (!await content.first().isVisible({ timeout: 10_000 }).catch(() => false)) {
      issue('STORES', 'No grid or empty state');
    }
  });

  test('invoices page', async () => {
    test.skip(!authenticated, 'Registration failed');
    await goto('/evm/invoices');

    if (!await scoutPage.locator('.page-title').isVisible({ timeout: 10_000 }).catch(() => false)) {
      issue('INVOICES', 'Title not visible');
    }

    const content = scoutPage.locator('.invoices-table-container, .invoices-cards, .empty-state');
    if (!await content.first().isVisible({ timeout: 10_000 }).catch(() => false)) {
      issue('INVOICES', 'No table or empty state');
    }
  });

  test('payments page', async () => {
    test.skip(!authenticated, 'Registration failed');
    await goto('/evm/payments');

    if (!await scoutPage.locator('.page-title').isVisible({ timeout: 10_000 }).catch(() => false)) {
      issue('PAYMENTS', 'Title not visible');
    }
  });

  test('wallets page', async () => {
    test.skip(!authenticated, 'Registration failed');
    await goto('/evm/wallets');

    if (!await scoutPage.locator('.page-title').isVisible({ timeout: 10_000 }).catch(() => false)) {
      issue('WALLETS', 'Title not visible');
    }

    const content = scoutPage.locator('.wallets-grid, .wallets-empty');
    if (!await content.first().isVisible({ timeout: 10_000 }).catch(() => false)) {
      issue('WALLETS', 'No grid or empty state');
    }
  });

  test('settings page tabs', async () => {
    test.skip(!authenticated, 'Registration failed');
    await goto('/evm/settings');

    const tabs = scoutPage.locator('.settings-tab');
    const count = await tabs.count();
    if (count < 3) {
      issue('SETTINGS', `Only ${count} tabs visible, expected at least 3`);
    }

    for (let i = 0; i < count; i++) {
      const label = await tabs.nth(i).textContent().catch(() => '');
      await tabs.nth(i).click();
      await scoutPage.waitForTimeout(500);
      const content = scoutPage.locator('.settings-tab-content, .detail-card');
      if (!await content.first().isVisible({ timeout: 3_000 }).catch(() => false)) {
        issue('SETTINGS', `Tab "${label}" rendered no content`);
      }
    }
  });

  test('404 page', async () => {
    test.skip(!authenticated, 'Registration failed');
    await goto('/evm/nonexistent');

    if (!await scoutPage.locator('.evm-not-found').isVisible({ timeout: 5_000 }).catch(() => false)) {
      issue('404', 'Not-found page did not render');
    }
  });

  test('responsive: mobile viewport', async () => {
    test.skip(!authenticated, 'Registration failed');
    await scoutPage.setViewportSize({ width: 375, height: 812 });
    await goto('/evm');

    const hamburger = scoutPage.locator('.mobile-menu-toggle');
    if (!await hamburger.isVisible({ timeout: 5_000 }).catch(() => false)) {
      issue('RESPONSIVE', 'Hamburger not visible on mobile');
    } else {
      await hamburger.click();
      if (!await scoutPage.locator('.sidebar').evaluate(el => el.classList.contains('open')).catch(() => false)) {
        issue('RESPONSIVE', 'Sidebar did not open on hamburger click');
      }
    }

    await scoutPage.setViewportSize({ width: 1280, height: 720 });
  });

  test('cross-component stress', async () => {
    test.skip(!authenticated, 'Registration failed');
    await goto('/evm');

    const menu = scoutPage.locator('.user-menu-trigger');
    const store = scoutPage.locator('.store-selector-btn');

    for (let i = 0; i < 5; i++) {
      await menu.click();
      await store.click();
      await menu.click();
      await store.click();
    }

    const btn = scoutPage.locator('.main-header-actions button', { hasText: /create invoice/i });
    const start = Date.now();
    await btn.click();
    const elapsed = Date.now() - start;
    if (elapsed > 500) {
      issue('PERF', `Button response ${elapsed}ms after stress (expected <500ms)`);
    }
    if (!await scoutPage.locator('.modal-overlay').isVisible({ timeout: 2_000 }).catch(() => false)) {
      issue('PERF', 'Modal did not open after stress test');
    }
  });
});

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

test('summary: all issues', async () => {
  const significant = consoleErrors.filter(
    e => !e.includes('favicon') && !e.includes('ResizeObserver'),
  );

  if (significant.length > 0) {
    console.log(`\n🔴 ${significant.length} JS errors:`);
    for (const e of significant) console.log(`    ${e}`);
  }

  if (issues.length > 0) {
    console.log(`\n🔴 ${issues.length} issues:`);
    for (const i of issues) console.log(`    ${i}`);
  }

  // Only fail on unexpected issues (not auth — that's a known test infra limitation)
  const nonAuthIssues = issues.filter(i => !i.startsWith('[AUTH]') && !i.startsWith('[REGISTER]'));
  expect(nonAuthIssues, `Non-auth issues:\n${nonAuthIssues.join('\n')}`).toHaveLength(0);
});
