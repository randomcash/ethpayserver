/**
 * Unit tests for the pure shaping logic behind the nightly visual review:
 * scripts/visual-review-report.mjs decides what counts as an error vs a
 * finding and how the two render. Nothing here exercises a browser or the
 * Anthropic API, so — like the route-coverage test in visual-review.spec.ts —
 * it isn't gated behind E2E_VISUAL_REVIEW and runs on every push.
 *
 * The property that matters most: a run that broke and a run that came back
 * clean must never render as the same string. That's the one thing this
 * report exists to guarantee, and it's the one thing a human skimming a
 * nightly issue has no other way to notice if it silently stopped holding.
 */
import { test, expect } from '@playwright/test';
import { groupByRoute, renderReport } from '../scripts/visual-review-report.mjs';

test.describe('groupByRoute', () => {
  test('buckets a captured shot by route and pushes a failed capture into errors, not routes', () => {
    const errors: unknown[] = [];
    const manifest = [
      { route: 'dashboard', path: '/evm', viewport: 'mobile', file: 'dashboard-mobile.png', error: null },
      { route: 'wallets', path: '/evm/wallets', viewport: 'mobile', file: null, error: 'timeout' },
    ];

    const routes = groupByRoute(manifest, errors);

    expect(routes.has('dashboard')).toBe(true);
    expect(routes.has('wallets')).toBe(false);
    expect(errors).toEqual([{ route: 'wallets', viewport: 'mobile', reason: 'capture failed: timeout' }]);
  });

  test('a route with one captured viewport and one failed viewport keeps the capture and records the failure', () => {
    const errors: unknown[] = [];
    const manifest = [
      { route: 'settings', path: '/evm/settings', viewport: 'mobile', file: 'settings-mobile.png', error: null },
      { route: 'settings', path: '/evm/settings', viewport: 'desktop', file: null, error: null },
    ];

    const routes = groupByRoute(manifest, errors);

    expect(routes.get('settings')?.shots).toHaveLength(1);
    expect(routes.get('settings')?.shots[0].viewport).toBe('mobile');
    expect(errors).toEqual([{ route: 'settings', viewport: 'desktop', reason: 'capture failed' }]);
  });
});

test.describe('renderReport', () => {
  test('a clean run and an errors-only run never render as the same report', () => {
    const clean = renderReport([], []);
    const incomplete = renderReport([], [{ route: 'wallets', viewport: 'mobile', reason: 'capture failed' }]);

    expect(clean).not.toBe(incomplete);
    expect(clean).toContain('No issues found.');
    expect(incomplete).not.toContain('No issues found.');
    expect(incomplete).toContain('could not be reviewed — treat this run as incomplete, not clean');
    expect(incomplete).toContain('No findings among the routes that were reviewed.');
  });

  test('findings and errors together render both, not one instead of the other', () => {
    const report = renderReport(
      [{ route: 'wallets', path: '/evm/wallets', viewport: 'mobile', what_is_wrong: 'raw decimal shown for balance' }],
      [{ route: 'settings', viewport: 'desktop', reason: 'capture failed' }],
    );

    expect(report).toContain('could not be reviewed — treat this run as incomplete, not clean');
    expect(report).toContain('settings (desktop): capture failed');
    expect(report).toContain('1 finding(s) across 1 route(s).');
    expect(report).toContain('/evm/wallets (wallets)');
    expect(report).toContain('raw decimal shown for balance');
  });

  test('a pipeline-level error with no route still renders distinctly from a clean run', () => {
    const report = renderReport([], [{ route: null, viewport: null, reason: 'no manifest — review did not run' }]);

    expect(report).toContain('pipeline: no manifest — review did not run');
    expect(report).not.toContain('No issues found.');
  });
});
