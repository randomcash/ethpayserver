import { test, expect } from '../fixtures/auth';
import { resetDatabase } from '../fixtures/db';
import { createStore } from '../fixtures/stores';
import { closeCreateInvoiceModal, createInvoiceModal, createInvoiceTrigger } from '../fixtures/invoices';
import type { Page } from '@playwright/test';

/**
 * Count global click listeners attached to the window.
 * Uses getEventListeners (Chrome DevTools Protocol) via CDP session.
 */
async function countWindowClickListeners(page: Page): Promise<number> {
  const client = await page.context().newCDPSession(page);
  const { result, exceptionDetails } = await client.send('Runtime.evaluate', {
    expression: `
      (() => {
        const listeners = getEventListeners(window);
        return listeners.click ? listeners.click.length : 0;
      })()
    `,
    returnByValue: true,
    // `getEventListeners` is a DevTools *console* helper, not a page global.
    // Without this it is simply undefined, the expression throws, and
    // `result.value` comes back undefined — which made this test compare
    // NaN <= 1 and fail with no hint as to why.
    includeCommandLineAPI: true,
  });
  await client.detach();

  if (exceptionDetails) {
    throw new Error(`getEventListeners failed: ${exceptionDetails.text}`);
  }
  if (typeof result.value !== 'number') {
    throw new Error(`expected a listener count, got ${JSON.stringify(result.value)}`);
  }
  return result.value;
}

test.describe('UI Interactions', () => {
  test.beforeAll(async () => {
    await resetDatabase();
  });

  test.describe('User menu', () => {
    test('opens and closes on trigger click', async ({ registeredPage: page }) => {
      await createStore(page, 'Menu Test Store');
      await page.goto('/evm');

      const trigger = page.locator('.user-menu-trigger');
      const dropdown = page.locator('.user-menu-dropdown');

      // Initially closed
      await expect(dropdown).not.toHaveClass(/open/);

      // Open
      await trigger.click();
      await expect(dropdown).toHaveClass(/open/);

      // Close
      await trigger.click();
      await expect(dropdown).not.toHaveClass(/open/);
    });

    test('closes on click outside', async ({ registeredPage: page }) => {
      await createStore(page, 'Outside Click Store');
      await page.goto('/evm');

      const trigger = page.locator('.user-menu-trigger');
      const dropdown = page.locator('.user-menu-dropdown');

      await trigger.click();
      await expect(dropdown).toHaveClass(/open/);

      // Click outside the menu (on the header search area)
      await page.locator('.main-header-search').click();
      await expect(dropdown).not.toHaveClass(/open/);
    });

    test('repeated open/close does not leak event listeners', async ({ registeredPage: page }) => {
      await createStore(page, 'Leak Test Store');
      await page.goto('/evm');

      const trigger = page.locator('.user-menu-trigger');
      const cycle = async (times: number) => {
        for (let i = 0; i < times; i++) {
          await trigger.click();
          await trigger.click();
        }
      };

      await cycle(20);
      const after20 = await countWindowClickListeners(page);
      await cycle(40);
      const after60 = await countWindowClickListeners(page);

      // What matters is that the count does not grow with the number of cycles
      // — before the fix it grew by one per open. The absolute number is not
      // the property: the app registers a fixed set of outside-click handlers
      // on first use and reuses them (measured: 2, flat across 20/40/60/80
      // cycles), so the old `<= 1` bound was asserting the wrong thing and only
      // ever passed because the helper was silently returning NaN.
      expect(after60).toBe(after20);
    });

    test('buttons remain responsive after menu cycling', async ({ registeredPage: page }) => {
      await createStore(page, 'Responsive Store');
      await page.goto('/evm');

      const trigger = page.locator('.user-menu-trigger');

      // Cycle menu 10 times
      for (let i = 0; i < 10; i++) {
        await trigger.click();
        await trigger.click();
      }

      // Create Invoice button must still respond promptly
      const createBtn = createInvoiceTrigger(page);
      const start = Date.now();
      await createBtn.click();
      const elapsed = Date.now() - start;

      // Should respond within 500ms — before the fix, accumulated
      // listeners would cause multi-second delays
      expect(elapsed).toBeLessThan(500);

      // Modal should have opened
      await expect(createInvoiceModal(page)).toBeVisible({ timeout: 2_000 });
    });

    test('Settings link navigates correctly after menu interactions', async ({ registeredPage: page }) => {
      await createStore(page, 'Nav Test Store');
      await page.goto('/evm');

      const trigger = page.locator('.user-menu-trigger');

      // Open/close a few times, then navigate via Settings
      for (let i = 0; i < 5; i++) {
        await trigger.click();
        await trigger.click();
      }

      await trigger.click();
      await page.locator('.user-menu-item', { hasText: /settings/i }).click();
      await page.waitForURL(/\/evm\/settings/);
    });
  });

  test.describe('Sidebar store dropdown', () => {
    test('opens and closes on click', async ({ registeredPage: page }) => {
      await createStore(page, 'Dropdown Store A');
      await page.goto('/evm');

      const selectorBtn = page.locator('.store-selector-btn');
      const dropdown = page.locator('.store-dropdown');

      await expect(dropdown).not.toHaveClass(/open/);

      await selectorBtn.click();
      await expect(dropdown).toHaveClass(/open/);

      await selectorBtn.click();
      await expect(dropdown).not.toHaveClass(/open/);
    });

    test('selecting a store closes dropdown and updates label', async ({ registeredPage: page }) => {
      for (const name of ['Selector Store A', 'Selector Store B']) {
        await createStore(page, name);
      }
      await page.goto('/evm');

      const selectorBtn = page.locator('.store-selector-btn');
      const dropdown = page.locator('.store-dropdown');

      await selectorBtn.click();
      await expect(dropdown).toHaveClass(/open/);

      await page.locator('.store-dropdown-item', { hasText: 'Selector Store B' }).click();
      await expect(dropdown).not.toHaveClass(/open/);
      await expect(page.locator('.store-selector-name')).toHaveText('Selector Store B');
    });

    test('rapid store switching does not freeze UI', async ({ registeredPage: page }) => {
      for (const name of ['Rapid Store A', 'Rapid Store B', 'Rapid Store C']) {
        await createStore(page, name);
      }
      await page.goto('/evm');

      const selectorBtn = page.locator('.store-selector-btn');
      const stores = ['Rapid Store A', 'Rapid Store B', 'Rapid Store C', 'All Stores'];

      // Rapidly switch through stores
      for (const name of stores) {
        await selectorBtn.click();
        await page.locator('.store-dropdown-item', { hasText: name }).click();
      }

      // UI should still be responsive — verify by opening the user menu
      const menuTrigger = page.locator('.user-menu-trigger');
      await menuTrigger.click();
      await expect(page.locator('.user-menu-dropdown')).toHaveClass(/open/);
    });
  });

  test.describe('Create Invoice modal', () => {
    test('opens and closes without blocking other elements', async ({ registeredPage: page }) => {
      await createStore(page, 'Modal Test Store');
      await page.goto('/evm');

      // Open modal
      await createInvoiceTrigger(page).click();
      await expect(createInvoiceModal(page)).toBeVisible();

      // Close modal (click overlay or close button)
      await closeCreateInvoiceModal(page);

      await expect(createInvoiceModal(page)).not.toBeVisible();

      // Other buttons should still work after modal closes
      const menuTrigger = page.locator('.user-menu-trigger');
      await menuTrigger.click();
      await expect(page.locator('.user-menu-dropdown')).toHaveClass(/open/);
    });

    test('repeated open/close does not degrade responsiveness', async ({ registeredPage: page }) => {
      await createStore(page, 'Modal Cycle Store');
      await page.goto('/evm');

      const createBtn = createInvoiceTrigger(page);

      for (let i = 0; i < 5; i++) {
        await createBtn.click();
        await expect(createInvoiceModal(page)).toBeVisible();

        // Close via overlay edge click
        await closeCreateInvoiceModal(page);

        await expect(createInvoiceModal(page)).not.toBeVisible();
      }

      // After cycling, navigation should still work
      await page.locator('.sidebar-nav a').first().click();
      await expect(page).not.toHaveURL(/\/evm$/);
    });
  });

  test.describe('Cross-component interaction', () => {
    test('all interactive elements respond after mixed interactions', async ({ registeredPage: page }) => {
      await createStore(page, 'Cross Component Store');
      await page.goto('/evm');

      const menuTrigger = page.locator('.user-menu-trigger');
      const selectorBtn = page.locator('.store-selector-btn');
      const createBtn = createInvoiceTrigger(page);

      // Interact with every component in sequence
      // 1. Toggle user menu
      await menuTrigger.click();
      await expect(page.locator('.user-menu-dropdown')).toHaveClass(/open/);
      await menuTrigger.click();

      // 2. Toggle store dropdown
      await selectorBtn.click();
      await expect(page.locator('.store-dropdown')).toHaveClass(/open/);
      await selectorBtn.click();

      // 3. Open/close invoice modal
      await createBtn.click();
      await expect(createInvoiceModal(page)).toBeVisible();
      await closeCreateInvoiceModal(page);

      // 4. Verify everything still works — open each again
      await menuTrigger.click();
      await expect(page.locator('.user-menu-dropdown')).toHaveClass(/open/);
      await menuTrigger.click();

      await selectorBtn.click();
      await expect(page.locator('.store-dropdown')).toHaveClass(/open/);
      await selectorBtn.click();

      await createBtn.click();
      await expect(createInvoiceModal(page)).toBeVisible();
    });

    test('rapid mixed interactions do not freeze the page', async ({ registeredPage: page }) => {
      await createStore(page, 'Rapid Mix Store');
      await page.goto('/evm');

      const menuTrigger = page.locator('.user-menu-trigger');
      const selectorBtn = page.locator('.store-selector-btn');

      // Rapidly alternate between components
      for (let i = 0; i < 5; i++) {
        await menuTrigger.click();
        await selectorBtn.click();
        await menuTrigger.click();
        await selectorBtn.click();
      }

      // Page should not be frozen — verify with a timed interaction
      const createBtn = createInvoiceTrigger(page);
      const start = Date.now();
      await createBtn.click();
      const elapsed = Date.now() - start;

      expect(elapsed).toBeLessThan(500);
      await expect(createInvoiceModal(page)).toBeVisible({ timeout: 2_000 });
    });
  });
});
