/**
 * Route lists shared between the nightly visual-review capture
 * (visual-review.spec.ts) and the static check that keeps them in sync with
 * scout.spec.ts's walk (route-coverage.spec.ts). Kept in their own module
 * rather than defined in either spec file: visual-review.spec.ts's
 * `test.beforeAll` requests the `browser` fixture, which Playwright resolves
 * — and launches Chromium for — the moment it's named as a parameter, even
 * though the hook body returns immediately when E2E_VISUAL_REVIEW isn't set.
 * Importing that file from the coverage check would drag that launch along
 * for a test that has no browser dependency of its own and needs none to run
 * at PR time.
 */
export const UNAUTHENTICATED_ROUTES: [string, string][] = [
  ['login', '/login'],
  ['register', '/register'],
];

export const AUTHENTICATED_ROUTES: [string, string][] = [
  ['dashboard', '/evm'],
  ['stores', '/evm/stores'],
  ['invoices', '/evm/invoices'],
  ['payments', '/evm/payments'],
  ['wallets', '/evm/wallets'],
  ['settings', '/evm/settings'],
];
