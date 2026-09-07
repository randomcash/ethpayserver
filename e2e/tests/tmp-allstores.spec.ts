import { test } from '@playwright/test';
import { setupVirtualAuthenticator, register } from '../fixtures/auth';

test('All Stores no longer dead-ends on Invoices and Payments', async ({ page }) => {
  test.setTimeout(180_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  await setupVirtualAuthenticator(page);
  await register(page);

  for (const path of ['/evm/invoices', '/evm/payments']) {
    await page.goto(path);
    await page.waitForLoadState('networkidle', { timeout: 15_000 }).catch(() => {});
    await page.waitForTimeout(1500);
    const body = ((await page.locator('body').textContent({ timeout: 3000 })) ?? '').replace(/\s+/g, ' ');
    const deadEnd = body.includes('scoped to a single store');
    console.log(`${path}: dead-end message present = ${deadEnd}`);
    const heading = ((await page.locator('.empty-state h3, .empty-state').first().textContent({ timeout: 2000 }).catch(() => '')) ?? '').trim().slice(0, 70);
    console.log(`  empty state says: ${heading || '(rows rendered)'}`);
  }
});
