import { test, expect } from '../fixtures/auth';
import { resetDatabase } from '../fixtures/db';
import { createStoreReadyForInvoices } from '../fixtures/payment-methods';
import { createInvoice, invoiceRows } from '../fixtures/invoices';

test.describe('Invoices', () => {
  test.beforeAll(async () => {
    await resetDatabase();
  });

  test('create invoice for store', async ({ registeredPage: page }) => {
    await createStoreReadyForInvoices(page, 'Invoice Store');
    await page.goto('/evm/invoices');

    await createInvoice(page, '0.01');

    // Creation lands on the invoice's own page; it reaches the list too
    await expect(page).toHaveURL(/\/evm\/invoices\/.+/);
    await page.goto('/evm/invoices');
    await expect(invoiceRows(page).first()).toBeVisible();
  });

  test('invoice appears in list', async ({ registeredPage: page }) => {
    await createStoreReadyForInvoices(page, 'List Invoice Store');
    await page.goto('/evm/invoices');

    await createInvoice(page, '0.05');

    // Reload the list and verify the invoice survived the round trip
    await page.goto('/evm/invoices');
    await expect(invoiceRows(page).first()).toBeVisible();
  });

  test('invoice detail shows correct data', async ({ registeredPage: page }) => {
    await createStoreReadyForInvoices(page, 'Detail Invoice Store');
    await page.goto('/evm/invoices');

    // createInvoice already leaves us on the detail page
    await createInvoice(page, '1.23');

    await expect(page.getByText('1.23').first()).toBeVisible();
  });
});
