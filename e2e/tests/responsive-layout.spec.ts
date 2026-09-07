import { test, expect } from '@playwright/test';
import { readFileSync } from 'fs';

/**
 * Layout regressions in the real stylesheet, with no server and no auth.
 *
 * `styles.css` is over 5000 lines and layers two design systems, so it contains
 * several pairs of rules with IDENTICAL specificity where the later one wins by
 * source order alone. That has produced three separate user-visible bugs, and
 * none of them could fail a normal test: the app compiles, renders, and is
 * simply wrong.
 *
 * These tests load the actual stylesheet against the actual class combinations
 * and assert computed style. No auth means no rate limit, and no server means
 * they run in milliseconds - which matters, because the alternative is noticing
 * from a screenshot.
 */
const CSS = readFileSync('../client/styles.css', 'utf8');

async function displays(page: import('@playwright/test').Page, html: string, sels: string[]) {
  await page.setContent(`<style>${CSS}</style>${html}`);
  const out: Record<string, string> = {};
  for (const s of sels) out[s] = await page.locator(s).evaluate((el) => getComputedStyle(el).display);
  return out;
}

/**
 * `.mobile-only { display: none }` sits at line ~2714; `.invoices-cards` and
 * `.payments-cards` set `display: flex` at ~2729 and ~3189. Same specificity,
 * so the component won and the card list rendered on desktop UNDER the table -
 * every table showed its rows twice.
 */
test.describe('responsive visibility', () => {
  const MARKUP = `
    <div class="payments-table-container desktop-only"><table class="payments-table"><tr><td>row</td></tr></table></div>
    <div class="payments-cards mobile-only"><div class="payment-card">card</div></div>
    <div class="invoices-cards mobile-only"><div>card</div></div>`;

  test('desktop shows the table and hides both card lists', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    const d = await displays(page, MARKUP, [
      '.payments-table-container', '.payments-cards', '.invoices-cards',
    ]);
    expect(d['.payments-table-container']).not.toBe('none');
    expect(d['.payments-cards'], 'payment cards must not double the table').toBe('none');
    expect(d['.invoices-cards'], 'invoice cards must not double the table').toBe('none');
  });

  test('mobile shows the card lists and hides the table', async ({ page }) => {
    await page.setViewportSize({ width: 500, height: 900 });
    const d = await displays(page, MARKUP, [
      '.payments-table-container', '.payments-cards',
    ]);
    expect(d['.payments-table-container']).toBe('none');
    expect(d['.payments-cards']).not.toBe('none');
  });
});

/**
 * `.form-group + .form-group { margin-top }` is a stacked-form rule that also
 * applied inside containers with their own `gap`, pushing every second field
 * 16px below the first. Reported twice: Amount vs Currency in the create-invoice
 * modal, and Account Created vs Last Login in Settings.
 */
test.describe('form field alignment', () => {
  const cases: [string, string][] = [
    ['.form-row', `<div class="form-row">
       <div class="form-group form-group-grow"><label class="form-label">A</label><input class="form-input"></div>
       <div class="form-group"><label class="form-label">B</label><select class="form-input"><option>x</option></select></div>
     </div>`],
    ['.settings-grid', `<div class="settings-grid">
       <div class="form-group"><label class="form-label">A</label><div class="form-static">v</div></div>
       <div class="form-group"><label class="form-label">B</label><div class="form-static">v</div></div>
     </div>`],
  ];

  for (const [container, markup] of cases) {
    test(`${container} aligns its fields on one line`, async ({ page }) => {
      await page.setViewportSize({ width: 1440, height: 900 });
      await page.setContent(`<style>${CSS}</style><div style="width:900px;padding:24px">${markup}</div>`);
      const groups = page.locator(`${container} > .form-group`);
      const a = await groups.nth(0).boundingBox();
      const b = await groups.nth(1).boundingBox();
      expect(Math.round(b!.y - a!.y), `${container} second field must not sit lower`).toBe(0);
    });
  }

  test('an input and a select are the same height side by side', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.setContent(
      `<style>${CSS}</style><div style="width:900px"><input class="form-input"><select class="form-input"><option>x</option></select></div>`);
    const i = await page.locator('input.form-input').boundingBox();
    const s = await page.locator('select.form-input').boundingBox();
    expect(Math.round(s!.height - i!.height), 'select must not sag beside an input').toBe(0);
  });
});
