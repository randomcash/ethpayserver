/**
 * Pure logic test for the filter scout's terminal assertion runs on the
 * issues it collects. No browser or server involved - it exists to prove,
 * mechanically, that the labels scout.spec.ts's new checks push into
 * issue() (NETWORK, ROUTE_DISCOVERY, RESPONSIVE, PANIC) actually fail the
 * run rather than only being printed, and that only the declared exceptions
 * do not.
 */
import { test, expect } from '@playwright/test';
import { gatingIssues } from '../fixtures/issues';

test('new check labels gate the run', () => {
  const issues = [
    '[AUTH] pre-login probe returned 401',
    '[REGISTER] registration flow returned 401',
    '[COVERAGE_GAP] payment detail page was not checked',
    '[NETWORK] GET /api/evm/invoices returned 500',
    '[ROUTE_DISCOVERY] dashboard sidebar rendered only 2 route link(s)',
    '[RESPONSIVE] table overflowed its card at mobile width',
    '[PANIC] WASM client panicked: unreachable',
  ];

  expect(gatingIssues(issues)).toEqual([
    '[NETWORK] GET /api/evm/invoices returned 500',
    '[ROUTE_DISCOVERY] dashboard sidebar rendered only 2 route link(s)',
    '[RESPONSIVE] table overflowed its card at mobile width',
    '[PANIC] WASM client panicked: unreachable',
  ]);
});

test('an empty issue list stays empty', () => {
  expect(gatingIssues([])).toEqual([]);
});
