import { test, expect } from '@playwright/test';
import { setupVirtualAuthenticator, register } from '../fixtures/auth';

test('the search box sends search= to the server, once, debounced', async ({ page }) => {
  test.setTimeout(240_000);
  const panics: string[] = [];
  page.on('pageerror', e => panics.push(e.message));
  const listCalls: string[] = [];
  page.on('request', r => {
    const u = r.url();
    if (u.includes('/api/invoices?') || u.includes('/api/payments?')) listCalls.push(u);
  });

  await page.setViewportSize({ width: 1440, height: 900 });
  await setupVirtualAuthenticator(page);
  await register(page);

  // A fresh account has no stores, and the list page never queries without one.
  await page.evaluate(async () => {
    const token = JSON.parse(localStorage.getItem('ps_session') || '{}').session_id;
    const r = await fetch('/api/stores', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${token}` },
      body: JSON.stringify({ name: 'search-probe' }),
    });
    if (!r.ok) throw new Error(`store create -> ${r.status}`);
  });

  await page.goto('/evm/invoices');
  await page.waitForLoadState('networkidle', { timeout: 20_000 }).catch(() => {});
  await page.waitForTimeout(1500);
  listCalls.length = 0;

  // Exactly the list search box. The page also has a "Search by tx hash..."
  // input, and a substring match picks that one instead.
  const box = page.locator('input[placeholder="Search invoices..."]');
  await box.click();
  for (const ch of 'abcdef') { await box.type(ch, { delay: 60 }); }   // 6 keystrokes, faster than the debounce
  await page.waitForTimeout(1500);

  const withSearch = listCalls.filter(u => u.includes('search='));
  console.log(`REQUESTS after typing 6 chars: total=${listCalls.length} withSearch=${withSearch.length}`);
  for (const u of withSearch) console.log(`  ${decodeURIComponent(u.replace(/^https?:\/\/[^/]+/, ''))}`);

  // Clearing must go back to an unfiltered query, not search=
  listCalls.length = 0;
  await box.fill('');
  await page.waitForTimeout(1500);
  const afterClear = listCalls.filter(u => u.includes('search='));
  console.log(`after clearing: calls=${listCalls.length} stillFiltered=${afterClear.length}`);
  console.log(`PANICS: ${panics.length ? panics.join(' | ') : 'none'}`);

  expect(withSearch.length, 'debounce must collapse 6 keystrokes into one request').toBe(1);
  expect(withSearch[0]).toContain('search=abcdef');
  expect(afterClear.length, 'clearing must drop the filter, not send search=').toBe(0);
  expect(panics).toEqual([]);
});
