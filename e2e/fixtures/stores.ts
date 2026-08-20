import { expect, type Page } from '@playwright/test';

/**
 * Create a store from `/evm/stores` and wait for its card to appear.
 *
 * The selectors are scoped to the "New Store" card on purpose: a bare
 * `.form-input` matches every field on the page — the Create Invoice modal is
 * mounted in the same layout and carries five of its own — so it is a
 * strict-mode violation, not a store-name input.
 *
 * This helper existed as four near-identical copies across the specs. That is
 * part of how it went stale unnoticed: nothing ran it, because the E2E job
 * carried `continue-on-error: true` (RCS-112).
 */
export async function createStore(page: Page, name: string): Promise<void> {
  await page.goto('/evm/stores');
  await page.locator('button', { hasText: /create store/i }).click();

  const form = page.locator('.detail-card', { hasText: 'New Store' });
  await form.getByPlaceholder('My Store').fill(name);
  await form.locator('.form-actions .btn-primary').click();

  await expect(page.locator('.store-card-name', { hasText: name })).toBeVisible();
}

/** Create a store and open its detail page. */
export async function createStoreAndOpen(page: Page, name: string): Promise<void> {
  await createStore(page, name);
  await page.locator('.store-card', { hasText: name }).click();
  await page.waitForURL(/\/evm\/stores\/.+/);
}
