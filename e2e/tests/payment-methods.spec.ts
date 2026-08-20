import { test, expect } from '../fixtures/auth';
import { resetDatabase } from '../fixtures/db';
import { createStoreAndOpen } from '../fixtures/stores';
import { addPaymentMethod, openPaymentMethodsTab } from '../fixtures/payment-methods';

/** Sepolia USDC. Any well-formed address works; a real one keeps it readable. */
const SEPOLIA_USDC = '0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238';

test.describe('Payment Methods', () => {
  test.beforeAll(async () => {
    await resetDatabase();
  });

  test('empty state shown for new store', async ({ registeredPage: page }) => {
    await createStoreAndOpen(page, 'Empty PM Store');
    await openPaymentMethodsTab(page);

    await expect(page.getByText('No payment methods configured')).toBeVisible();
    await expect(page.locator('.payment-method-row')).toHaveCount(0);
  });

  test('add native payment method', async ({ registeredPage: page }) => {
    await createStoreAndOpen(page, 'Native PM Store');
    await openPaymentMethodsTab(page);

    await addPaymentMethod(page, { symbol: 'ETH' });

    const row = page.locator('.payment-method-row');
    await expect(row).toHaveCount(1);
    await expect(row.locator('.payment-method-symbol')).toHaveText('ETH');
    await expect(row.locator('.payment-method-type')).toHaveText('Native');
  });

  test('add ERC20 payment method', async ({ registeredPage: page }) => {
    await createStoreAndOpen(page, 'ERC20 PM Store');
    await openPaymentMethodsTab(page);

    await addPaymentMethod(page, {
      symbol: 'USDC',
      tokenAddress: SEPOLIA_USDC,
      decimals: '6',
    });

    const row = page.locator('.payment-method-row');
    await expect(row).toHaveCount(1);
    await expect(row.locator('.payment-method-symbol')).toHaveText('USDC');
    await expect(row.locator('.payment-method-type')).toHaveText('ERC20');
  });

  test('toggle payment method enabled/disabled', async ({ registeredPage: page }) => {
    await createStoreAndOpen(page, 'Toggle PM Store');
    await openPaymentMethodsTab(page);
    await addPaymentMethod(page);

    // The status control is a button badge, not a checkbox — the old spec
    // looked for `input[type="checkbox"]`, found nothing, and asserted nothing.
    const status = page.locator('.payment-method-row .badge');
    await expect(status).toHaveText('Enabled');

    await status.click();
    await expect(status).toHaveText('Disabled');

    // and it survives a round trip to the server
    await page.reload();
    await openPaymentMethodsTab(page);
    await expect(page.locator('.payment-method-row .badge')).toHaveText('Disabled');
  });

  test('delete payment method', async ({ registeredPage: page }) => {
    await createStoreAndOpen(page, 'Delete PM Store');
    await openPaymentMethodsTab(page);
    await addPaymentMethod(page);
    await expect(page.locator('.payment-method-row')).toHaveCount(1);

    await page.locator('.payment-method-row button', { hasText: 'Delete' }).click();

    await expect(page.locator('.payment-method-row')).toHaveCount(0);
    await expect(page.getByText('No payment methods configured')).toBeVisible();
  });
});
