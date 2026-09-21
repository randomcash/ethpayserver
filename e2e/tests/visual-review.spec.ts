/**
 * Nightly visual review: screenshots every route scout.spec.ts already knows
 * how to reach, at a mobile and a desktop viewport, so an agent can look at
 * each one and report what a deterministic assertion cannot catch — clipped
 * content, an unlabelled control, a raw decimal where a formatted amount
 * belongs. This file only captures; scripts/visual-review.mjs does the
 * looking, against the manifest written below.
 *
 * Advisory-only, not part of the gating suite: ci.yml's `e2e` job runs
 * `npx playwright test` with no filter, so without the gate below this file
 * would run on every push and capture screenshots nobody reviews. Run it
 * explicitly with E2E_VISUAL_REVIEW=true, as the scheduled workflow does.
 */
import { test as base, type Page } from '@playwright/test';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { setupVirtualAuthenticator, register } from '../fixtures/auth';

const RUN = process.env.E2E_VISUAL_REVIEW === 'true';
const SKIP_REASON = 'Nightly-only: set E2E_VISUAL_REVIEW=true to run (see scripts/visual-review.mjs)';

const VISUAL_DIR = path.join('test-results', 'visual');

const VIEWPORTS = [
  { name: 'mobile', width: 375, height: 812 },
  { name: 'desktop', width: 1280, height: 720 },
] as const;

// The routes scout.spec.ts is already known to reach unauthenticated and
// after a single passkey registration. Kept in step with that file rather
// than re-deriving a route list some other way.
const UNAUTHENTICATED_ROUTES: [string, string][] = [
  ['login', '/login'],
  ['register', '/register'],
];

const AUTHENTICATED_ROUTES: [string, string][] = [
  ['dashboard', '/evm'],
  ['stores', '/evm/stores'],
  ['invoices', '/evm/invoices'],
  ['payments', '/evm/payments'],
  ['wallets', '/evm/wallets'],
  ['settings', '/evm/settings'],
];

interface ManifestEntry {
  route: string;
  path: string;
  viewport: string;
  file: string | null;
  error: string | null;
}

const manifest: ManifestEntry[] = [];

let sharedPage: Page;

const test = base.extend({});
test.describe.configure({ mode: 'serial' });

test.beforeAll(async ({ browser }) => {
  fs.mkdirSync(VISUAL_DIR, { recursive: true });
  const ctx = await browser.newContext();
  sharedPage = await ctx.newPage();
});

test.afterAll(async () => {
  fs.writeFileSync(path.join(VISUAL_DIR, 'manifest.json'), JSON.stringify(manifest, null, 2));
  console.log(`\n=== VISUAL REVIEW: ${manifest.length} capture(s), ${manifest.filter(m => m.error).length} error(s) ===`);
  for (const m of manifest) {
    console.log(m.error ? `  [ERROR] ${m.route} (${m.viewport}): ${m.error}` : `  ${m.route} (${m.viewport}) -> ${m.file}`);
  }
  await sharedPage?.context().close();
});

// One navigation, one screenshot. Never throws — a route that fails to load
// is a manifest entry with an error, not a reason to abandon the rest of the
// walk. Serial mode skips every later test after a thrown error, and this
// spec exists to name problems, not to stop at the first one.
async function capture(route: string, urlPath: string) {
  for (const vp of VIEWPORTS) {
    const file = `${route}-${vp.name}.png`;
    const entry: ManifestEntry = { route, path: urlPath, viewport: vp.name, file: null, error: null };
    try {
      await sharedPage.setViewportSize({ width: vp.width, height: vp.height });
      await sharedPage.goto(urlPath);
      await sharedPage.waitForLoadState('networkidle', { timeout: 15_000 }).catch(() => {});
      await sharedPage.screenshot({ path: path.join(VISUAL_DIR, file), fullPage: true });
      entry.file = file;
    } catch (err) {
      entry.error = err instanceof Error ? err.message : String(err);
    }
    manifest.push(entry);
  }
}

test.describe('Unauthenticated routes', () => {
  test('capture login and register', async () => {
    test.skip(!RUN, SKIP_REASON);
    for (const [route, urlPath] of UNAUTHENTICATED_ROUTES) {
      await capture(route, urlPath);
    }
  });
});

test.describe('Authenticated routes', () => {
  // One registration, reused for every authenticated capture — the same
  // shape as scout's `register with passkey` test. Registering once per
  // route the way some of the older specs do hits the auth-tier rate limit
  // (5 req/min/IP) against a remote origin; see e2e/README.md.
  test('register once, then capture the sidebar routes', async () => {
    test.skip(!RUN, SKIP_REASON);

    await setupVirtualAuthenticator(sharedPage);
    try {
      await register(sharedPage);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      manifest.push({
        route: 'register',
        path: '/register',
        viewport: 'n/a',
        file: null,
        error: message,
      });
      // Registration gates every authenticated capture below — record each
      // route as uncaptured rather than letting it vanish from the manifest.
      // A vanished route and a clean route both read as "nothing to report";
      // an explicit error entry is the only way to tell them apart.
      for (const [route, urlPath] of AUTHENTICATED_ROUTES) {
        manifest.push({
          route,
          path: urlPath,
          viewport: 'n/a',
          file: null,
          error: `not captured — passkey registration failed: ${message}`,
        });
      }
      return;
    }

    for (const [route, urlPath] of AUTHENTICATED_ROUTES) {
      await capture(route, urlPath);
    }
  });
});
