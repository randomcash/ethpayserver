import { test } from '@playwright/test';
import { setupVirtualAuthenticator, register } from '../fixtures/auth';

test('network status and tables after deploy', async ({ page }) => {
  test.setTimeout(180_000);
  const panics: string[] = [];
  page.on('pageerror', e => panics.push(e.message));
  await page.setViewportSize({ width: 1440, height: 1200 });
  await setupVirtualAuthenticator(page);
  await register(page);

  await page.goto('/evm');
  await page.waitForLoadState('networkidle', { timeout: 20_000 }).catch(() => {});
  await page.waitForTimeout(4000);
  const net = ((await page.locator('.network-list, .activity-note').last().textContent({ timeout: 3000 }).catch(() => '')) ?? '').replace(/\s+/g,' ').trim();
  console.log(`NETWORK STATUS: ${net.slice(0, 140)}`);
  const rows = await page.locator('.network-row, .network-list > *').count();
  console.log(`NETWORK ROWS: ${rows}`);
  await page.screenshot({ path: 'final-dash.png', fullPage: false });

  await page.goto('/evm/payments');
  await page.waitForLoadState('networkidle', { timeout: 15_000 }).catch(() => {});
  await page.waitForTimeout(1500);
  const body = ((await page.locator('body').textContent({ timeout: 3000 })) ?? '').replace(/\s+/g,' ');
  console.log(`PAGINATION LEAK present: ${body.includes('total_pages') || body.includes('Next page on:click')}`);
  const tables = await page.locator('.payments-table-container:visible').count();
  const cards = await page.locator('.payments-cards:visible').count();
  console.log(`VISIBLE desktop-table=${tables} mobile-cards=${cards}`);
  console.log(`PANICS: ${panics.length ? panics.join(' | ') : 'none'}`);
});
