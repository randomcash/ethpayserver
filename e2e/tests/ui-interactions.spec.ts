import { test, expect } from '../fixtures/auth';
import { resetDatabase } from '../fixtures/db';
import type { Page } from '@playwright/test';

async function createStore(page: Page, name: string): Promise<void> {
  await page.goto('/evm/stores');
  await page.locator('button', { hasText: /create store/i }).click();
  await page.locator('.form-input').fill(name);
  await page.locator('.form-actions .btn-primary').click();
  await expect(page.locator('.store-card-name', { hasText: name })).toBeVisible();
}

/**
 * Count global click listeners attached to the window.
 * Uses getEventListeners (Chrome DevTools Protocol) via CDP session.
 */
async function countWindowClickListeners(page: Page): Promise<number> {
  const client = await page.context().newCDPSession(page);
  const { result } = await client.send('Runtime.evaluate', {
    expression: `
      (() => {
        const listeners = getEventListeners(window);
        return listeners.click ? listeners.click.length : 0;
      })()
    `,
    returnByValue: true,
  });
  await client.detach();
  return result.value as number;
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
      const baselineCount = await countWindowClickListeners(page);

      // Cycle the menu 20 times
      for (let i = 0; i < 20; i++) {
        await trigger.click();
        await trigger.click();
      }

      const afterCount = await countWindowClickListeners(page);
      // Allow at most 1 new listener (the close-on-outside-click handler).
      // Before the fix this would grow by 20.
      expect(afterCount - baselineCount).toBeLessThanOrEqual(1);
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
      const createBtn = page.locator('button', { hasText: /create invoice/i });
      const start = Date.now();
      await createBtn.click();
      const elapsed = Date.now() - start;

      // Should respond within 500ms — before the fix, accumulated
      // listeners would cause multi-second delays
      expect(elapsed).toBeLessThan(500);

      // Modal should have opened
      await expect(page.locator('.modal-overlay, .create-invoice-modal')).toBeVisible({ timeout: 2_000 });
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
      await page.locator('button', { hasText: /create invoice/i }).click();
      await expect(page.locator('.modal-overlay, .create-invoice-modal')).toBeVisible();

      // Close modal (click overlay or close button)
      const closeBtn = page.locator('.modal-close, button[aria-label="Close"]').first();
      if (await closeBtn.isVisible({ timeout: 1_000 }).catch(() => false)) {
        await closeBtn.click();
      } else {
        await page.locator('.modal-overlay').click({ position: { x: 5, y: 5 } });
      }

      await expect(page.locator('.modal-overlay, .create-invoice-modal')).not.toBeVisible();

      // Other buttons should still work after modal closes
      const menuTrigger = page.locator('.user-menu-trigger');
      await menuTrigger.click();
      await expect(page.locator('.user-menu-dropdown')).toHaveClass(/open/);
    });

    test('repeated open/close does not degrade responsiveness', async ({ registeredPage: page }) => {
      await createStore(page, 'Modal Cycle Store');
      await page.goto('/evm');

      const createBtn = page.locator('button', { hasText: /create invoice/i });

      for (let i = 0; i < 5; i++) {
        await createBtn.click();
        await expect(page.locator('.modal-overlay, .create-invoice-modal')).toBeVisible();

        // Close via overlay edge click
        const closeBtn = page.locator('.modal-close, button[aria-label="Close"]').first();
        if (await closeBtn.isVisible({ timeout: 1_000 }).catch(() => false)) {
          await closeBtn.click();
        } else {
          await page.locator('.modal-overlay').click({ position: { x: 5, y: 5 } });
        }

        await expect(page.locator('.modal-overlay, .create-invoice-modal')).not.toBeVisible();
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
      const createBtn = page.locator('button', { hasText: /create invoice/i });

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
      await expect(page.locator('.modal-overlay, .create-invoice-modal')).toBeVisible();
      const closeBtn = page.locator('.modal-close, button[aria-label="Close"]').first();
      if (await closeBtn.isVisible({ timeout: 1_000 }).catch(() => false)) {
        await closeBtn.click();
      } else {
        await page.locator('.modal-overlay').click({ position: { x: 5, y: 5 } });
      }

      // 4. Verify everything still works — open each again
      await menuTrigger.click();
      await expect(page.locator('.user-menu-dropdown')).toHaveClass(/open/);
      await menuTrigger.click();

      await selectorBtn.click();
      await expect(page.locator('.store-dropdown')).toHaveClass(/open/);
      await selectorBtn.click();

      await createBtn.click();
      await expect(page.locator('.modal-overlay, .create-invoice-modal')).toBeVisible();
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
      const createBtn = page.locator('button', { hasText: /create invoice/i });
      const start = Date.now();
      await createBtn.click();
      const elapsed = Date.now() - start;

      expect(elapsed).toBeLessThan(500);
      await expect(page.locator('.modal-overlay, .create-invoice-modal')).toBeVisible({ timeout: 2_000 });
    });
  });
});
