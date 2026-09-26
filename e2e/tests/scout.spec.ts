/**
 * Live-site scout: tests unauthenticated flows and (if auth succeeds)
 * authenticated pages. Designed for E2E_REMOTE=true.
 *
 * Run with:  E2E_REMOTE=true npx playwright test tests/scout.spec.ts
 */
import { appendFileSync } from 'node:fs';

import { test as base, expect, type Page, type ConsoleMessage } from '@playwright/test';
import { parseEther } from 'viem';
import { setupVirtualAuthenticator, isClientPanic } from '../fixtures/auth';
import { api } from '../fixtures/api';
import { seedPaymentForInvoice } from '../fixtures/db';
import { createInvoice } from '../fixtures/invoices';
import { gatingIssues } from '../fixtures/issues';
import { createStoreReadyForInvoices } from '../fixtures/payment-methods';

// db.ts's own fixtures no-op under E2E_REMOTE (no DB reachable from a live
// deployment) - scoutPage runs there too, so anything that needs a direct
// insert has to check this itself rather than assume a DB connection exists.
const isRemote = process.env.E2E_REMOTE === 'true';

let scoutPage: Page;
const issues: string[] = [];
const consoleErrors: string[] = [];
let authenticated = false;
// `authenticated` above is sticky for the rest of the suite once registration
// succeeds - every `test.skip(!authenticated, ...)` depends on that. The
// response listener below needs a narrower, non-sticky signal instead: is
// there a session on THIS browser context right now. Those two diverge
// exactly during the checkout branch of walkRoutes, which clears cookies and
// localStorage to simulate an anonymous visit while `authenticated` (correctly)
// stays true for the rest of the suite - without a separate flag, the
// /api/auth/me exclusion below reads the stale `authenticated`, the checkout
// visit's genuine, expected 401 doesn't match it, and gets misreported as a
// NETWORK issue on every run that reaches the checkout branch.
let sessionPresent = false;
/**
 * The account `register with passkey` created, captured from the recovery
 * screen before it is dismissed - same source as `RecoveryCredentials.accountId`
 * in fixtures/auth.ts. Module scope because `afterAll` needs it after the test
 * that set it has already gone out of scope.
 */
let createdUserId: string | null = null;
/**
 * Set when the recovery screen was shown - so an account genuinely exists -
 * but reading `.ps-recovery-account-id-value` failed anyway. `createdUserId`
 * alone cannot tell that apart from "no account was created", and the two
 * need opposite handling: the first is a real leak with no other record of
 * it (the screen shows exactly once), the second is a correct no-op.
 */
let idCaptureFailedAfterRegistration = false;

// Every issue() call - a panic, a network failure, an overflow, anything -
// lands in this one array, and the `summary: all issues` test at the bottom
// of the file asserts it empty except for the declared non-gating labels in
// gatingIssues() (fixtures/issues.ts) - AUTH/REGISTER and COVERAGE_GAP, each
// a known test-infra limitation rather than a bug. There is no separate
// "just print" path: calling issue() is what fails the run, for every caller
// in this file, including the network and responsive checks below.
function issue(label: string, detail: string) {
  issues.push(`[${label}] ${detail}`);
}

// Referenced by the response listener below (to scope which 404s are
// expected) and by PLACEHOLDER_ROUTES further down. Declared this early so
// the listener doesn't read like it's guessing at what "expected" means.
const PLACEHOLDER_ID = '00000000-0000-0000-0000-000000000000';

const test = base.extend({});
test.describe.configure({ mode: 'serial' });

test.beforeAll(async ({ browser }) => {
  const ctx = await browser.newContext();
  scoutPage = await ctx.newPage();
  // A client panic is an issue(), not just a logged line. scout has collected
  // console errors since it was written and only ever PRINTED them, which is
  // why two panics on every registration survived for months in a
  // suite that reported no issues. The summary asserts on non-auth issues, so
  // recording it here is what makes it fail.
  //
  // Only panics. Ordinary console errors are noisy on this page (404s, the
  // checkout WebSocket handshake) and failing on those would get this muted.
  const notePanic = (text: string) => {
    if (isClientPanic(text)) issue('PANIC', `WASM client panicked: ${text.split('\n')[0].trim()}`);
  };
  scoutPage.on('console', (msg: ConsoleMessage) => {
    if (msg.type() !== 'error') return;
    consoleErrors.push(msg.text());
    notePanic(msg.text());
  });
  scoutPage.on('pageerror', (err: Error) => {
    consoleErrors.push(`UNCAUGHT: ${err.message}`);
    notePanic(err.message);
  });

  // The console listener above only sees a problem if the client logs one -
  // and a page that renders perfectly can still sit on top of an API that
  // answered every call with a 500. Nothing here read the network before,
  // which is a wider version of the exact gap that let two panics ship: a
  // failure that leaves no console trace still needs to be a finding, not a
  // silently-passing page.
  scoutPage.on('response', (resp) => {
    const url = new URL(resp.url());
    if (!url.pathname.startsWith('/api/')) return;
    const status = resp.status();
    const req = resp.request();

    // A 404 is the expected shape of "not found", but only for the
    // placeholder-id routes below - each one calls an API path that embeds
    // PLACEHOLDER_ID literally (e.g. `/api/payments/{id}`), the same way the
    // nonexistent-invoice checkout test already relied on before this
    // listener existed. Scoped to that id rather than excluding 404 for every
    // call: an authenticated page whose own query 404s (a broken join, a
    // dangling foreign key) is exactly the class of bug this listener exists
    // to catch, and a blanket exclusion would drop it silently on every page
    // in the walk, not just the six that are supposed to 404.
    //
    // A 401/403 before any session exists is the same kind of expected shape
    // - the client probes /api/auth/me on every load, and that probe is
    // supposed to fail pre-login. Scoped to that exact path, the same way the
    // 404 exclusion above is scoped to the placeholder id rather than
    // excluding the status everywhere: a 401/403 from any OTHER endpoint
    // pre-login (a public config fetch, a CSRF token endpoint) would be a
    // regression, not the expected probe, and a blanket exclusion would drop
    // it silently the same way a blanket 404 exclusion would.
    if (status === 404 && url.pathname.includes(PLACEHOLDER_ID)) return;
    if ((status === 401 || status === 403) && !sessionPresent && url.pathname === '/api/auth/me') return;

    if (status >= 500) {
      issue('NETWORK', `${req.method()} ${url.pathname} -> ${status}`);
    } else if (status === 429) {
      issue('NETWORK', `${req.method()} ${url.pathname} -> 429 (rate limited)`);
    } else if (status === 401 || status === 403) {
      issue('NETWORK', `${req.method()} ${url.pathname} -> ${status} ${sessionPresent ? 'while authenticated' : 'before authentication'}`);
    } else if (status >= 400) {
      issue('NETWORK', `${req.method()} ${url.pathname} -> ${status}`);
    }
  });
  // Connection-level failures (refused, DNS, reset) never reach 'response' at
  // all, so they need their own listener rather than a status check. But
  // walkRoutes does a hard goto() per route, and tearing a page down to
  // navigate to the next one cancels whatever that page still had in flight
  // (a poll, a session-refresh ping, a slow list fetch) - Chromium reports
  // that to Playwright as 'requestfailed' with errorText 'net::ERR_ABORTED',
  // not because anything broke but because the page it belonged to is gone.
  // Excluded by errorText rather than dropped as a class: an abort caused by
  // something other than our own navigation would still report under this
  // name, but the alternative - flagging every cross-page navigation in the
  // crawl - makes the run fail unpredictably on browser-normal behavior,
  // which gets a suite ignored or disabled exactly as fast as one that
  // silently prints.
  scoutPage.on('requestfailed', (req) => {
    const url = new URL(req.url());
    if (!url.pathname.startsWith('/api/')) return;
    const errorText = req.failure()?.errorText ?? 'unknown error';
    if (errorText === 'net::ERR_ABORTED') return;
    issue('NETWORK', `${req.method()} ${url.pathname} failed: ${errorText}`);
  });
});

/**
 * Remove the account `register with passkey` just created.
 *
 * Without this, every run leaves a passkey-only account behind forever: there
 * is no session to reach `DELETE /users/me` with once this hook's own browser
 * context closes, and nobody holds the passkey to sign back in later. Cleanup
 * lives here, next to the registration that creates the account, rather than
 * in a separate script nobody is forced to run - a teardown living somewhere
 * else is exactly how this residue accumulated the first time.
 *
 * `E2E_API_TOKEN` (a `server_admin` bearer token) is optional: scout also runs
 * locally and against `E2E_REMOTE=false` without one, where leaving a throwaway
 * local account behind costs nothing. Skipped silently in that case, not
 * treated as a failure.
 *
 * A genuine leak (the id-capture failure below, or the delete itself failing)
 * throws rather than only pushing to `issues`. `issues` is what `summary: all
 * issues` asserts against, but that test - like every other test in this file
 * - runs *before* `afterAll`, which is the only place this function is called
 * from. Recording a leak here without throwing would report it to nobody: the
 * one test that reads `issues` has already passed by the time this runs,
 * exactly the "logged but doesn't fail the build" gap `synthetic-payment.spec.ts`'s
 * `afterEach` avoids by throwing for the same reason.
 */
async function cleanupCreatedAccount() {
  if (!createdUserId) {
    if (idCaptureFailedAfterRegistration) {
      // A real account exists - the recovery screen was shown - but its id
      // was never captured, so nothing below can reach it to delete it and
      // no later run can recognize it either: `scout.spec.ts` never sets an
      // email or a wallet, so this is indistinguishable from any other
      // passkey-only signup. A leak with no other record of it has to
      // surface the same way a cleanup failure does, not disappear before
      // that path is even reached.
      const msg =
        'Registration completed and the recovery screen was shown, but reading the ' +
        'account id from .ps-recovery-account-id-value failed - the account exists but ' +
        'cannot be identified or cleaned up here or by sweep-e2e-accounts.mjs.';
      console.log(`::error title=Scout account id capture failed::${msg}`);
      if (process.env.GITHUB_STEP_SUMMARY) {
        appendFileSync(process.env.GITHUB_STEP_SUMMARY, `### ❌ Account id capture failed\n\n${msg}\n`);
      }
      issue('CLEANUP', msg);
      throw new Error(msg);
    }
    return;
  }
  const token = process.env.E2E_API_TOKEN;
  if (!token) {
    console.log(`account ${createdUserId} left in place - set E2E_API_TOKEN to sweep it here`);
    return;
  }
  try {
    await api(`/admin/users/${createdUserId}`, { method: 'DELETE', token });
    console.log(`cleaned up account ${createdUserId}`);
  } catch (err) {
    const msg =
      `Failed to clean up scout account ${createdUserId}: ${err}. It is still on the ` +
      `server and will stay there - delete it with \`node scripts/sweep-e2e-accounts.mjs --execute\`.`;
    console.log(`::error title=Scout account leaked::${msg}`);
    if (process.env.GITHUB_STEP_SUMMARY) {
      appendFileSync(process.env.GITHUB_STEP_SUMMARY, `### ❌ Account leaked\n\n${msg}\n`);
    }
    issue('CLEANUP', msg);
    throw new Error(msg);
  }
}

test.afterAll(async () => {
  if (consoleErrors.length > 0) {
    console.log('\n=== JS CONSOLE ERRORS ===');
    for (const e of consoleErrors) console.log(`  ${e}`);
  }
  // Captured rather than let propagate immediately: the issues log below and
  // closing the browser context must still happen on a cleanup failure, and
  // rethrowing after both is what actually fails this hook - which is what
  // fails the run, since no test left running can see this happen otherwise.
  let cleanupError: unknown;
  try {
    await cleanupCreatedAccount();
  } catch (err) {
    cleanupError = err;
  }
  if (issues.length > 0) {
    console.log('\n=== ISSUES FOUND ===');
    for (const i of issues) console.log(`  ${i}`);
  } else {
    console.log('\n=== NO ISSUES FOUND ===');
  }
  await scoutPage?.context().close();
  if (cleanupError) throw cleanupError;
});

async function goto(path: string) {
  await scoutPage.goto(path);
  await scoutPage.waitForLoadState('networkidle', { timeout: 15_000 }).catch(() => {});
}

/**
 * `goto` for the authenticated tests, which all assume a live session.
 *
 * When the session is gone the app renders the Sign In page, so every
 * `.sidebar-link` / `.user-menu-trigger` the test then reaches for is simply
 * absent. playwright.config sets no `actionTimeout`, so the first `click()` on
 * one of those waits until the TEST times out — 60s of nothing, and because
 * this suite is serial, every later test is abandoned. That is how one lost
 * session cost 11 of 22 tests and reported no cause; the scout exists to name
 * problems in one line, not to hang on them.
 *
 * So check once, here, and turn it into a normal skip: the issue is recorded,
 * the rest of the suite still runs, and the summary still prints.
 */
async function gotoAuthed(path: string) {
  await goto(path);
  const signIn = scoutPage.locator('h1', { hasText: /sign in/i });
  if (await signIn.isVisible({ timeout: 2_000 }).catch(() => false)) {
    issue('SESSION', `Bounced to Sign In on ${path} — session was lost after registration`);
    test.skip(true, 'Session lost — see the SESSION issue in the summary');
  }
}

const DESKTOP_VIEWPORT = { width: 1280, height: 720 };
const MOBILE_VIEWPORT = { width: 375, height: 812 };

/**
 * The router itself (payserver-client's app/mod.rs) is never available to
 * read here: CI's e2e job runs the published image pinned in
 * ops/client-image.pin, and scout's own remote mode runs against a live
 * deployment - in neither case is the Rust source on disk. The DOM is the
 * only thing this test can actually inspect, so route discovery reads every
 * link the app renders, anywhere on the page, not only ones tagged
 * `.sidebar-link` - a page linked from a dashboard card or a settings tab is
 * covered the same as one linked from the sidebar, without this file
 * changing. Restricted to the authenticated app's own path prefixes so a
 * mailto:, an external footer link, or a download href doesn't get treated
 * as a route to navigate to - and so `/login`/`/register` don't turn up
 * here: they're walked by the Unauthenticated tests above, and this crawl
 * runs against a live session, where landing on either is exactly the
 * signed-out state `gotoAuthed` exists to catch, not a route to add to it.
 *
 * Plugin pages (`/evm/plugins/:id/:path`) are a second exception, handled by
 * `discoverPluginRoutes` below rather than this crawl: whether a plugin's
 * link happens to be in the DOM this scan reaches depends on rendering
 * timing and page layout, where the server's own plugin registry does not.
 *
 * This is still a DOM crawl, not a router read: a route this scout account
 * can't reach - gated behind a role it doesn't have, a feature flag, or a
 * data-dependent empty state that renders no link - stays invisible the same
 * way it did before this file existed. Give it its own data source, the way
 * `discoverPluginRoutes` does, rather than assuming a link will appear here.
 *
 * `MIN_DASHBOARD_LINKS` below narrows that gap for the one set of routes
 * where "how many links should be there" is actually knowable without a
 * router read: the sidebar's unconditional top-level pages. It cannot say
 * anything about a route reachable only through some other page's
 * conditionally-rendered link - that's still the open gap this comment
 * describes - but it does mean the sidebar's own routes can no longer drop
 * out silently.
 */
const ROUTE_HREF_PATTERN = /^\/(evm(\/|$)|checkout\/)/;

// gotoAuthed/goto only wait for network-idle, not for the WASM client to
// finish hydrating, so anything that inspects the DOM right after a
// navigation - a link scan, an overflow measurement - can read the loading
// skeleton instead of the real page, with nothing to say it happened.
//
// Waiting for the page's own content to appear would conflate two things
// that need different treatment: hydration running slow, and a page -
// checkout, say - that genuinely renders no nav links, or no overflow, once
// hydrated. Both look identical from "did the expected thing show up".
// #initial-loader (index.html) doesn't have that ambiguity: it's removed
// synchronously the moment payserver-client's mount_app() runs, on every hard
// navigation, regardless of what the page it mounts contains. So waiting for
// it to detach is a wait for "hydration finished", not "this page happens to
// have a link" or "this page happens to overflow" - a timeout here means
// hydration didn't complete, and is worth a finding rather than a silent
// pass-through, at every call site that measures the DOM.
async function waitForHydration(context: string, label: 'ROUTE_DISCOVERY' | 'RESPONSIVE'): Promise<boolean> {
  const hydrated = await scoutPage
    .locator('#initial-loader')
    .waitFor({ state: 'detached', timeout: 5_000 })
    .then(() => true)
    .catch(() => false);
  if (!hydrated) {
    issue(label, `${context} did not finish hydrating within 5s - measurement likely inaccurate`);
  }
  return hydrated;
}

async function discoverLinkedRoutes(): Promise<string[]> {
  // evaluateAll snapshots whatever matches right now and does not auto-wait
  // the way a locator assertion would, so the hydration wait above has to run
  // first or this can undercount links - down to zero.
  await waitForHydration(scoutPage.url(), 'ROUTE_DISCOVERY');
  const hrefs = await scoutPage
    .locator('a[href]')
    .evaluateAll((els) => els.map((el) => el.getAttribute('href') ?? ''));
  return [
    ...new Set(
      hrefs
        .map((href) => href.split(/[?#]/)[0])
        .filter((href) => ROUTE_HREF_PATTERN.test(href)),
    ),
  ];
}

interface PluginPagesResponse {
  plugins?: { id: string; slug: string; pages?: { path: string }[] }[];
}

/**
 * Plugin pages the DOM crawl above cannot reliably find: a freshly
 * registered scout account has no data of its own, but plugins are
 * installed per deployment, not per account, so `/api/plugins` - the same
 * endpoint the client's own sidebar calls to build `PluginLinks` - lists
 * every page a plugin declared for this session's role. Reading that list
 * directly is what "enumerate from the router" means for a route the
 * client's router only describes as a wildcard (`/plugins/:id/:path`): the
 * concrete instances come from server-declared data, not from Rust source
 * this test can read, so the data source has to be the same one the real
 * client renders from.
 *
 * The billing plugin is the ticket's named example, but nothing here is
 * billing-specific - a new plugin with a new page is covered the moment it
 * is installed and enabled, without this file changing.
 *
 * This request goes through `scoutPage.request`, Playwright's
 * `APIRequestContext` - it never touches the browser's network stack, so the
 * `page.on('response')` listener in beforeAll never sees it. A broken
 * plugins endpoint would otherwise fail silently: `discoverPluginRoutes`
 * would return `[]`, the walk would visit zero billing pages, and both
 * route-coverage tests would pass clean having checked nothing. Every exit
 * path below is therefore its own issue() rather than a quiet empty array -
 * including a well-formed `200 {}` (no `plugins` field, a schema change this
 * test would otherwise coalesce into "no plugins" via `?? []`) and a
 * well-formed `200 {"plugins": []}` once CI's fixture plugin is accounted
 * for: `.github/workflows/ci.yml` seeds one merchant-visible page before the
 * server starts specifically so this account always has at least one to
 * find, so an empty result here means discovery broke, not that nothing was
 * installed.
 *
 * `list_plugin_pages` (server/src/api/plugins.rs) requires `AuthenticatedUser`,
 * which reads only an `Authorization: Bearer` header - never a cookie, per the
 * same extractor the checkout comment above cites. `scoutPage.request` shares
 * the browser context's cookies but not its localStorage, so without this the
 * token the real client reads from `ps_session` on every boot never reaches
 * this call: it 401s every time, on every run, and the billing pages this
 * function exists to find are never discovered - not an edge case, the normal
 * case. Reading the session id back out of localStorage the same way the
 * client's own `use_auth` hook does (`session_id.to_string()` as the token) is
 * what makes this call actually authenticate as the real client would.
 */
async function discoverPluginRoutes(): Promise<string[]> {
  const sessionRaw = await scoutPage.evaluate(() => localStorage.getItem('ps_session'));
  // Every other exit path in this function turns a broken assumption into an
  // issue() rather than a throw - this one shouldn't be the exception just
  // because it runs first. An unparsable ps_session would otherwise crash the
  // whole route-coverage test instead of just failing to authenticate this
  // one request.
  let sessionId: string | undefined;
  if (sessionRaw) {
    try {
      sessionId = (JSON.parse(sessionRaw) as { session_id?: string }).session_id;
    } catch {
      issue('NETWORK', 'ps_session in localStorage was not valid JSON - could not authenticate the plugin discovery request');
    }
  }
  const resp = await scoutPage.request
    .get('/api/plugins', sessionId ? { headers: { Authorization: `Bearer ${sessionId}` } } : {})
    .catch(() => null);
  if (!resp) {
    issue('NETWORK', 'GET /api/plugins failed: no response');
    return [];
  }
  if (!resp.ok()) {
    issue('NETWORK', `GET /api/plugins -> ${resp.status()}`);
    return [];
  }
  const body: PluginPagesResponse | null = await resp.json().catch(() => null);
  if (body === null) {
    issue('NETWORK', 'GET /api/plugins returned a body that could not be parsed as JSON');
    return [];
  }
  if (!('plugins' in body)) {
    issue('NETWORK', 'GET /api/plugins response has no "plugins" field - response shape may have changed');
    return [];
  }
  // A real browser builds this link from the slug (short, human-chosen -
  // `/billing/subscriptions`), never the id (`/cash.random.billing/...`);
  // `get_page` accepts either, but only the slug shape is what the client
  // actually requests, so that's the shape worth walking.
  //
  // A plugin missing its `pages` field entirely is a schema deviation, not
  // the same thing as a plugin that legitimately declares `pages: []` - the
  // aggregate `routes.length === 0` check below only catches every plugin
  // losing the field at once, so one plugin (the billing one, say) silently
  // dropping `pages` while another still reports some would otherwise pass
  // clean having walked nothing for it. Each occurrence gets its own issue().
  const routes = (body.plugins ?? []).flatMap((plugin) => {
    if (plugin.pages === undefined) {
      issue('ROUTE_DISCOVERY', `plugin "${plugin.slug}" has no "pages" field - response shape may have changed`);
      return [];
    }
    return plugin.pages.map((page) => `/evm/plugins/${plugin.slug}/${page.path}`);
  });
  if (routes.length === 0) {
    if (isRemote) {
      // The CI fixture plugin that guarantees this account has at least one
      // page (see the comment above) is seeded by `.github/workflows/ci.yml`
      // and is not something a live deployment is guaranteed to have - a
      // remote run against a real merchant account with no plugins installed
      // would otherwise fail here for a reason that has nothing to do with
      // route discovery being broken.
      issue('COVERAGE_GAP', 'GET /api/plugins listed zero pages for this account - E2E_REMOTE has no guarantee of a fixture plugin, so the billing surface was not checked');
    } else {
      issue('ROUTE_DISCOVERY', 'GET /api/plugins listed zero pages for this account - the billing surface would go unwalked');
    }
  }
  return routes;
}

// A route reachable only through a real record's id is invisible to any DOM
// scan: a freshly registered scout account starts with none of those
// records, so the app never renders a link to one. Store, wallet, invoice and
// payment detail (and, through the invoice, checkout) are no longer on this
// list - `route coverage: desktop` below seeds one real record of each before
// crawling, so discoverLinkedRoutes finds the resulting store-card/
// wallet-card/invoice-row/payment-row links itself, the same "read what the
// app actually rendered" approach discoverPluginRoutes already uses for
// plugin pages. Visiting only the not-found branch of those routes would have
// proven nothing about the page a merchant or customer actually sees.
//
// A real payment only exists after evmmonitor observes an on-chain
// settlement, which needs a funded wallet and a live RPC endpoint this walk
// has neither of - see `seedPaymentForInvoice`'s comment in fixtures/db.ts for
// why a direct DB insert stands in for it instead, and only where a DB
// connection actually exists (not under E2E_REMOTE). The placeholder-id
// payment route below still runs everywhere: it is the not-found-branch
// check, kept independently of whether a real payment gets seeded.
const PLACEHOLDER_ROUTES = [`/evm/payments/${PLACEHOLDER_ID}`, '/evm/nonexistent'];

// layout.rs renders these five links unconditionally in the sidebar for any
// authenticated account - no role, plan or data-state gate around any of
// them - so unlike a stores/invoices/wallets page's own link count (which
// legitimately varies with what the account has), a dashboard crawl finding
// fewer than this many is never "this account just doesn't have that yet."
// It means one dropped out of the nav - a role gate or feature flag added
// later - and a DOM crawl has no way to notice a link's *absence* on its own.
// This is the bounded, cheap substitute for reading the router directly: it
// can't tell you about a route with no rendered link anywhere (that still
// needs a data source of its own, the way discoverPluginRoutes has), but it
// does turn "the sidebar quietly lost a route" from nothing into an issue().
const MIN_DASHBOARD_LINKS = 5;

// Safety valve, not an expected ceiling: this app has nowhere near this many
// distinct routes, so hitting it means link discovery found something
// unbounded (e.g. per-row links once this account has data) rather than that
// coverage is actually this wide.
const MAX_ROUTES = 40;

const OVERFLOW_TOLERANCE_PX = 1;

// Shared by walkRoutes below and by the unauthenticated login/register mobile
// check further down - both are "does the page rendered at this viewport fit
// it", and duplicating the evaluate() would let the two drift.
async function recordOverflow(path: string) {
  // A loading skeleton is very unlikely to overflow horizontally regardless
  // of how broken the real hydrated page is, so this has to wait for
  // hydration itself - it cannot rely on a caller's crawl step to have done
  // it, since the mobile walk below measures overflow with crawl disabled.
  await waitForHydration(path, 'RESPONSIVE');
  // null (not 0) on failure: this check's only job is to catch overflow, so a
  // page that crashed mid-evaluate must not read the same as a page that
  // measured cleanly at zero.
  const overflowPx = await scoutPage
    .evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)
    .catch(() => null);
  if (overflowPx === null) {
    issue('RESPONSIVE', `${path} overflow check could not run`);
  } else if (overflowPx > OVERFLOW_TOLERANCE_PX) {
    issue('RESPONSIVE', `${path} overflows horizontally by ${overflowPx}px at mobile width`);
  }
}

interface WalkOptions {
  /** Keep discovering new links from each page visited. Off for the mobile
   * pass, which reuses the desktop pass's already-complete route set instead
   * of re-crawling a site that hasn't changed shape between the two. */
  crawl?: boolean;
  /** Flag pages that render wider than the viewport - the shape of "a table
   * overflowing its card", the concrete bug the mobile pass exists for. */
  checkOverflow?: boolean;
}

async function walkRoutes(seedRoutes: string[], opts: WalkOptions = {}): Promise<string[]> {
  const { crawl = true, checkOverflow = false } = opts;
  const visited = new Set<string>();
  const queue = [...seedRoutes];
  const order: string[] = [];
  let maxRoutesIssued = false;

  while (queue.length > 0) {
    const path = queue.shift()!;
    if (visited.has(path)) continue;
    visited.add(path);
    order.push(path);

    // The only public route in the mix; everything else needs a session. A
    // real customer reaches checkout with no session at all, but scoutPage is
    // the same context that just registered as the merchant. The server
    // extracts auth from an `Authorization: Bearer` header, never a cookie
    // (server/src/api/extractors.rs), and the client reads that token from
    // localStorage on boot (ui-kit's AuthContext::load_session), not from a
    // cookie either - clearing cookies alone (an earlier version of this fix)
    // leaves the merchant's token in localStorage, so the freshly-booted
    // client on this "anonymous" visit would authenticate itself anyway.
    // Snapshot and clear localStorage the same way cookies are handled, and
    // restore both right after, so a branch keyed off "is there any session
    // on this browser" gets exercised the way a customer would trigger it,
    // without logging the rest of the walk out.
    if (path.startsWith('/checkout/')) {
      const cookies = await scoutPage.context().cookies();
      const storage = await scoutPage.evaluate(() => JSON.stringify(localStorage));
      await scoutPage.context().clearCookies();
      await scoutPage.evaluate(() => localStorage.clear());
      // Genuinely anonymous for the duration of this navigation - the
      // /api/auth/me exclusion above needs to know that, not the suite-wide
      // `authenticated` flag, which stays true throughout.
      sessionPresent = false;
      // scoutPage is shared, serial-mode state for the rest of the file - if
      // goto() or either restore step below throws, an un-restored session
      // would silently strip auth from every subsequent route (including the
      // mobile pass, which reuses this same context), turning one checkout
      // failure into a cascade of misleading "not authenticated" findings.
      // The restore must run even when the navigation itself is what failed.
      try {
        await goto(path);
      } finally {
        await scoutPage.context().addCookies(cookies);
        await scoutPage.evaluate((serialized) => {
          for (const [key, value] of Object.entries(JSON.parse(serialized) as Record<string, string>)) {
            localStorage.setItem(key, value);
          }
        }, storage);
        sessionPresent = authenticated;
      }
    } else {
      await gotoAuthed(path);
    }

    if (checkOverflow) {
      await recordOverflow(path);
    }

    if (crawl) {
      if (visited.size < MAX_ROUTES) {
        for (const href of await discoverLinkedRoutes()) {
          if (!visited.has(href) && !queue.includes(href)) queue.push(href);
        }
      } else if (!maxRoutesIssued) {
        // Hitting the cap means either an unbounded source of links (e.g.
        // per-row links once this account has data) or coverage genuinely
        // cut off - either way that's a fact worth surfacing, not a silent
        // stop. Guarded to fire once per walk rather than on every
        // remaining page in the queue.
        maxRoutesIssued = true;
        issue('ROUTE_DISCOVERY', `Hit MAX_ROUTES (${MAX_ROUTES}) while crawling - coverage may be incomplete`);
      }
    }
  }

  return order;
}

// ---------------------------------------------------------------------------
// Unauthenticated flows — these always run
// ---------------------------------------------------------------------------

test.describe('Unauthenticated', () => {
  test('login page renders correctly', async () => {
    await goto('/login');

    // Page should show sign-in form
    // `.first()`: 'Sign In' matches both the heading and the submit button, and
    // isVisible() on a multi-match locator throws a strict-mode violation that the
    // catch below would record as a phantom issue.
    const heading = scoutPage.getByText('Sign In').first();
    if (!await heading.isVisible({ timeout: 10_000 }).catch(() => false)) {
      issue('LOGIN', 'Sign In heading not visible');
    }

    // Auth tabs should be visible
    const walletTab = scoutPage.locator('.ps-auth-tab', { hasText: /wallet/i });
    const passkeyTab = scoutPage.locator('.ps-auth-tab', { hasText: /passkey/i });
    if (!await walletTab.isVisible({ timeout: 3_000 }).catch(() => false)) {
      issue('LOGIN', 'Wallet tab not visible');
    }
    if (!await passkeyTab.isVisible({ timeout: 3_000 }).catch(() => false)) {
      issue('LOGIN', 'Passkey tab not visible');
    }

    // "Create one" link
    const createLink = scoutPage.getByText('Create one').first();
    if (!await createLink.isVisible({ timeout: 3_000 }).catch(() => false)) {
      issue('LOGIN', '"Create one" registration link not visible');
    }
  });

  test('register page renders correctly', async () => {
    await goto('/register');

    const heading = scoutPage.getByText('Create Account');
    if (!await heading.isVisible({ timeout: 10_000 }).catch(() => false)) {
      issue('REGISTER', 'Create Account heading not visible');
    }

    const walletTab = scoutPage.locator('.ps-auth-tab', { hasText: /wallet/i });
    const passkeyTab = scoutPage.locator('.ps-auth-tab', { hasText: /passkey/i });
    if (!await walletTab.isVisible({ timeout: 3_000 }).catch(() => false)) {
      issue('REGISTER', 'Wallet tab not visible');
    }
    if (!await passkeyTab.isVisible({ timeout: 3_000 }).catch(() => false)) {
      issue('REGISTER', 'Passkey tab not visible');
    }

    // Switch to passkey tab
    if (await passkeyTab.isVisible().catch(() => false)) {
      await passkeyTab.click();
      const createBtn = scoutPage.locator('.ps-passkey-button');
      if (!await createBtn.isVisible({ timeout: 3_000 }).catch(() => false)) {
        issue('REGISTER', 'Create Passkey button not visible after switching to passkey tab');
      }
    }

    // "Sign in" link
    const signIn = scoutPage.getByText('Sign in').first();
    if (!await signIn.isVisible({ timeout: 3_000 }).catch(() => false)) {
      issue('REGISTER', '"Sign in" link not visible');
    }
  });

  // Every UI bug reported by hand on 2026-09-20 was on a phone, and /login and
  // /register are two of the original nine routes. The authenticated
  // `route coverage: mobile` pass further down can't cover either: both pages
  // redirect to the dashboard the instant a session exists (see ui-kit's
  // LoginPage/RegisterPage), so visiting them post-login just re-checks the
  // dashboard under a different URL. Checked here instead, while this suite
  // still has no session - the only point at which either page actually
  // renders.
  test('login and register render without horizontal overflow on mobile', async () => {
    await scoutPage.setViewportSize(MOBILE_VIEWPORT);
    try {
      await goto('/login');
      await recordOverflow('/login');
      await goto('/register');
      await recordOverflow('/register');
    } finally {
      await scoutPage.setViewportSize(DESKTOP_VIEWPORT);
    }
  });

  test('unauthenticated access redirects to login', async () => {
    await goto('/evm/stores');
    try {
      await expect(scoutPage).toHaveURL(/\/login/, { timeout: 5_000 });
    } catch {
      issue('AUTH_REDIRECT', `Expected redirect to /login, got ${scoutPage.url()}`);
    }
  });

  test('health endpoint is up', async () => {
    const resp = await scoutPage.request.get('/health/live');
    if (resp.status() !== 200) {
      issue('HEALTH', `Health endpoint returned ${resp.status()}`);
    }
  });

  test('API returns 401 for unauthenticated requests', async () => {
    const resp = await scoutPage.request.get('/api/auth/me');
    if (resp.status() !== 401) {
      issue('API', `/api/auth/me returned ${resp.status()} instead of 401`);
    }
  });

  test('checkout page for nonexistent invoice shows error', async () => {
    await goto('/checkout/00000000-0000-0000-0000-000000000000');

    const error = scoutPage.locator('.checkout-error');
    const loading = scoutPage.locator('.checkout-loading');
    // Should show error or loading (then error)
    await scoutPage.waitForTimeout(3_000);
    if (await loading.isVisible().catch(() => false)) {
      // Still loading after 3s — might be stuck
      issue('CHECKOUT', 'Checkout page still loading after 3s for nonexistent invoice');
    }
    // Error state is correct behavior here
  });

  test('login page wallet tab shows MetaMask prompt', async () => {
    await goto('/login');
    const walletTab = scoutPage.locator('.ps-auth-tab', { hasText: /wallet/i });
    if (await walletTab.isVisible({ timeout: 3_000 }).catch(() => false)) {
      await walletTab.click();
      // Should show "No Ethereum wallet detected" since we're headless
      const noWallet = scoutPage.getByText(/no.*wallet.*detected|install.*metamask/i);
      if (!await noWallet.isVisible({ timeout: 3_000 }).catch(() => false)) {
        issue('LOGIN', 'No wallet-not-detected message when no wallet extension present');
      }
    }
  });
});

// ---------------------------------------------------------------------------
// Authentication attempt
// ---------------------------------------------------------------------------

test.describe('Auth & Authenticated', () => {
  test('register with passkey', async () => {
    await setupVirtualAuthenticator(scoutPage);
    await goto('/register');

    // Switch to passkey tab
    const passkeyTab = scoutPage.locator('.ps-auth-tab', { hasText: /passkey/i });
    await passkeyTab.click();

    // Fill username if present
    const usernameInput = scoutPage.locator('.ps-passkey-form input:not([type="hidden"]):not([type="checkbox"])');
    if (await usernameInput.isVisible({ timeout: 1_000 }).catch(() => false)) {
      // Random suffix as well as a timestamp: this spec targets shared
      // environments, where a same-millisecond collision would surface as a
      // confusing duplicate-account failure rather than a clean one.
      const unique = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
      await usernameInput.fill(`scout_${unique}`);
    }

    // Take screenshot before clicking
    await scoutPage.screenshot({ path: 'test-results/scout-before-register.png' });

    // Click create passkey
    const createBtn = scoutPage.locator('.ps-passkey-button');
    await createBtn.click();

    // No blind sleep here: the race below waits on a real signal instead, and
    // 3s of fixed delay only ate budget the diagnostics need.

    // Check for errors
    const errorAlert = scoutPage.locator('.ps-auth-error, .ps-alert-error, [class*="error"]');
    const errorTexts: string[] = [];
    const errorCount = await errorAlert.count();
    for (let i = 0; i < errorCount; i++) {
      const text = await errorAlert.nth(i).textContent({ timeout: 2_000 }).catch(() => '');
      if (text && text.trim()) errorTexts.push(text.trim());
    }
    if (errorTexts.length > 0) {
      issue('REGISTER', `Error after passkey creation: ${errorTexts.join(' | ')}`);
    }

    // Handle the recovery step, which registration now REQUIRES -
    // "Skip for Now" is gone from the default flow, so the confirm path is the
    // only way through.
    //
    // The 5s budget this used was the actual cause of the remote failure in #56,
    // reported there as "passkey registration does not establish a session".
    // Registration succeeds; it parks on the recovery screen, and against a
    // remote server the passkey round trip takes longer than 5s to get there.
    // Both isVisible checks then returned false, the step was silently skipped,
    // registration never completed, and the run ended on Sign In - which reads
    // exactly like a failed login. fixtures/auth.ts already carries a comment
    // about this same 5s trap; scout kept it.
    //
    // Verified against live testnet on 2026-09-06: with a proper wait, start ->
    // complete -> /auth/me all return 200 and the session is established.
    // Budget matters. playwright.config allows 30s per test locally and 60s
    // remotely, and this test still needs ~10s afterwards for the URL assertion
    // and screenshots. A 30s wait would consume the local budget and time the
    // test out INSTEAD of producing the issue() diagnostics scout exists for -
    // and CI runs this suite against localhost on every push to testnet. 15s is
    // comfortably past the round trip that defeated the old 5s wait.
    //
    // No "Skip for Now" branch: that button is gone, so matching it could
    // only burn the timeout, and its broad `button` selector risked clicking an
    // unrelated button once the dashboard had rendered.
    const savedButton = scoutPage.locator('.ps-button-primary', { hasText: /written it down/i });
    const settledUrl = /\/(evm)?$/;

    await Promise.race([
      scoutPage.waitForURL(settledUrl, { timeout: 15_000 }).catch(() => {}),
      savedButton.waitFor({ state: 'visible', timeout: 15_000 }).catch(() => {}),
    ]);

    if (await savedButton.isVisible().catch(() => false)) {
      // Capture before dismissing: this screen is shown exactly once, and a
      // passkey-only account has no email or wallet to identify it by
      // afterwards. Same selector as `RecoveryCredentials.accountId` in
      // fixtures/auth.ts.
      createdUserId =
        (
          await scoutPage
            .locator('.ps-recovery-account-id-value')
            .textContent({ timeout: 2_000 })
            .catch(() => null)
        )?.trim() || null;
      if (!createdUserId) idCaptureFailedAfterRegistration = true;

      await savedButton.click();
      await scoutPage.locator('.ps-checkbox').check();
      await scoutPage.locator('.ps-button-primary', { hasText: /complete setup/i }).click();
    }

    // Take screenshot after
    await scoutPage.screenshot({ path: 'test-results/scout-after-register.png' });

    // Check if we reached the dashboard
    try {
      await expect(scoutPage).toHaveURL(/\/(evm)?$/, { timeout: 10_000 });
      authenticated = true;
      sessionPresent = true;
    } catch {
      issue('REGISTER', `Registration did not redirect to dashboard. Final URL: ${scoutPage.url()}`);

      // Try to see what page we ended up on
      const pageText = await scoutPage.locator('body').textContent({ timeout: 2_000 }).catch(() => '');
      if (pageText?.includes('Sign In')) {
        issue('REGISTER', 'Ended up on login page — session not created after registration');
      }
    }
  });

  // --- Authenticated tests (skip if registration failed) ---------------------

  test('dashboard loads', async () => {
    test.skip(!authenticated, 'Registration failed');
    await gotoAuthed('/evm');

    if (!await scoutPage.locator('.dashboard-header, .page-title').first().isVisible({ timeout: 10_000 }).catch(() => false)) {
      issue('DASHBOARD', 'Dashboard header not visible');
    }

    // Metrics
    const cards = await scoutPage.locator('.metric-card').count();
    if (cards !== 4) issue('DASHBOARD', `Expected 4 metric cards, got ${cards}`);

    // Charts
    if (!await scoutPage.locator('.charts-section').isVisible({ timeout: 5_000 }).catch(() => false)) {
      issue('DASHBOARD', 'Charts section missing');
    }

    // Activity
    if (!await scoutPage.locator('.activity-section').isVisible({ timeout: 5_000 }).catch(() => false)) {
      issue('DASHBOARD', 'Activity section missing');
    }

    // WebSocket indicator
    // Explicit timeout, like every textContent in this file. playwright.config
    // sets no actionTimeout, so an element that never appears makes textContent
    // wait until the TEST times out - the .catch never runs, and scout reports a
    // 60s timeout instead of the issue() it exists to record. That is what made
    // `dashboard loads` fail: .ws-indicator-label is absent on this deployment.
    const wsLabel = await scoutPage
      .locator('.ws-indicator-label')
      .textContent({ timeout: 2_000 })
      .catch(() => '');
    if (wsLabel === 'Offline') {
      issue('WEBSOCKET', 'Shows Offline on live site');
    }
  });

  test('sidebar navigation works', async () => {
    test.skip(!authenticated, 'Registration failed');

    const routes: [string, RegExp][] = [
      ['Invoices', /\/evm\/invoices/],
      ['Payments', /\/evm\/payments/],
      ['Stores', /\/evm\/stores/],
      ['Wallets', /\/evm\/wallets/],
      ['Settings', /\/evm\/settings/],
      ['Dashboard', /\/(evm)?$/],
    ];

    for (const [label, pattern] of routes) {
      const link = scoutPage.locator('.sidebar-link', { hasText: label });
      if (!await link.isVisible({ timeout: 3_000 }).catch(() => false)) {
        issue('SIDEBAR', `"${label}" link not visible`);
        continue;
      }
      await link.click();
      try {
        await expect(scoutPage).toHaveURL(pattern, { timeout: 5_000 });
      } catch {
        issue('SIDEBAR', `"${label}" did not navigate. URL: ${scoutPage.url()}`);
      }
    }
  });

  test('user menu interactions', async () => {
    test.skip(!authenticated, 'Registration failed');
    await gotoAuthed('/evm');

    const trigger = scoutPage.locator('.user-menu-trigger');
    const dropdown = scoutPage.locator('.user-menu-dropdown');

    // Open
    await trigger.click();
    if (!await dropdown.evaluate(el => el.classList.contains('open')).catch(() => false)) {
      issue('USER_MENU', 'Did not open');
    }

    // Close via trigger
    await trigger.click();
    await scoutPage.waitForTimeout(200);
    if (await dropdown.evaluate(el => el.classList.contains('open')).catch(() => true)) {
      issue('USER_MENU', 'Did not close on second click');
    }

    // Close via outside click
    await trigger.click();
    // Not `.main-header-search` - it no longer exists. The heading is outside
    // the menu and is not itself a control. `.dashboard-title` because this
    // runs on /evm; `.page-header` is a list-page class.
    await scoutPage.locator('.dashboard-title').click({ timeout: 5_000 });
    await scoutPage.waitForTimeout(300);
    if (await dropdown.evaluate(el => el.classList.contains('open')).catch(() => true)) {
      issue('USER_MENU', 'Did not close on outside click');
    }
  });

  test('event listener leak check', async () => {
    test.skip(!authenticated, 'Registration failed');
    await gotoAuthed('/evm');

    const trigger = scoutPage.locator('.user-menu-trigger');
    const client = await scoutPage.context().newCDPSession(scoutPage);

    const before = await client.send('Runtime.evaluate', {
      expression: `(()=>{const l=getEventListeners(window);return l.click?l.click.length:0})()`,
      returnByValue: true,
      // `getEventListeners` is a DevTools *console* helper, not a page global.
      // Without this it is undefined, the expression throws, `result.value`
      // comes back undefined, and the `leaked > 1` check below silently
      // compares NaN — making the LISTENER_LEAK issue unreachable.
      includeCommandLineAPI: true,
    });
    const baseline = before.result.value as number;

    for (let i = 0; i < 20; i++) {
      await trigger.click();
      await trigger.click();
    }

    const after = await client.send('Runtime.evaluate', {
      expression: `(()=>{const l=getEventListeners(window);return l.click?l.click.length:0})()`,
      returnByValue: true,
      // Console-helper API, as above.
      includeCommandLineAPI: true,
    });
    await client.detach();

    // A non-numeric result means the CDP expression failed rather than that
    // nothing leaked; report it instead of comparing NaN and passing.
    if (typeof baseline !== 'number' || typeof after.result.value !== 'number') {
      issue('LISTENER_LEAK', 'getEventListeners did not return a count — check could not run');
      return;
    }

    const leaked = (after.result.value as number) - baseline;
    if (leaked > 1) {
      issue('LISTENER_LEAK', `${leaked} click listeners leaked after 20 menu cycles`);
    }
  });

  test('create invoice modal', async () => {
    test.skip(!authenticated, 'Registration failed');
    await gotoAuthed('/evm');

    // Scoped to the header: the modal's own submit button carries the same
    // label, so an unscoped match is a strict-mode violation.
    await scoutPage.locator('.main-header-actions button', { hasText: /create invoice/i }).click();
    if (!await scoutPage.locator('.modal-overlay').isVisible({ timeout: 3_000 }).catch(() => false)) {
      issue('CREATE_INVOICE', 'Modal did not open');
      return;
    }

    // Check fields
    for (const id of ['#ci-amount', '#ci-currency', '#ci-expiration']) {
      if (!await scoutPage.locator(id).isVisible({ timeout: 2_000 }).catch(() => false)) {
        issue('CREATE_INVOICE', `Field ${id} not visible`);
      }
    }

    // Submit empty. The modal now DISABLES the submit button while the amount
    // is empty, which is the validation working - and is stricter than what
    // this test was written to check. click() waits for the element to become
    // enabled, so clicking it anyway just burned the full 30s test budget and
    // was the last failure in the suite.
    await scoutPage.locator('#ci-amount').fill('');
    const submit = scoutPage.locator('.modal .ps-btn-primary');
    if (!await submit.isDisabled({ timeout: 2_000 }).catch(() => false)) {
      // Not disabled, so the guard has to be on submit instead: click and
      // confirm the modal stays open.
      await submit.click({ timeout: 5_000 }).catch(() => {});
      await scoutPage.waitForTimeout(500);
      if (!await scoutPage.locator('.modal-overlay').isVisible().catch(() => false)) {
        issue('CREATE_INVOICE', 'Modal closed with empty amount — no validation');
      }
    }

    // Close
    await scoutPage
      .locator('.modal-overlay')
      .click({ position: { x: 5, y: 5 }, timeout: 5_000 })
      .catch(() => issue('CREATE_INVOICE', 'Modal did not close on overlay click'));
  });

  test('stores page', async () => {
    test.skip(!authenticated, 'Registration failed');
    await gotoAuthed('/evm/stores');

    if (!await scoutPage.locator('.page-title').isVisible({ timeout: 10_000 }).catch(() => false)) {
      issue('STORES', 'Title not visible');
    }

    const content = scoutPage.locator('.stores-grid, .stores-empty, .store-card');
    if (!await content.first().isVisible({ timeout: 10_000 }).catch(() => false)) {
      issue('STORES', 'No grid or empty state');
    }
  });

  test('invoices page', async () => {
    test.skip(!authenticated, 'Registration failed');
    await gotoAuthed('/evm/invoices');

    if (!await scoutPage.locator('.page-title').isVisible({ timeout: 10_000 }).catch(() => false)) {
      issue('INVOICES', 'Title not visible');
    }

    const content = scoutPage.locator('.invoices-table-container, .invoices-cards, .empty-state');
    if (!await content.first().isVisible({ timeout: 10_000 }).catch(() => false)) {
      issue('INVOICES', 'No table or empty state');
    }
  });

  test('payments page', async () => {
    test.skip(!authenticated, 'Registration failed');
    await gotoAuthed('/evm/payments');

    if (!await scoutPage.locator('.page-title').isVisible({ timeout: 10_000 }).catch(() => false)) {
      issue('PAYMENTS', 'Title not visible');
    }
  });

  test('wallets page', async () => {
    test.skip(!authenticated, 'Registration failed');
    await gotoAuthed('/evm/wallets');

    if (!await scoutPage.locator('.page-title').isVisible({ timeout: 10_000 }).catch(() => false)) {
      issue('WALLETS', 'Title not visible');
    }

    const content = scoutPage.locator('.wallets-grid, .wallets-empty');
    if (!await content.first().isVisible({ timeout: 10_000 }).catch(() => false)) {
      issue('WALLETS', 'No grid or empty state');
    }
  });

  test('settings page tabs', async () => {
    test.skip(!authenticated, 'Registration failed');
    await gotoAuthed('/evm/settings');

    const tabs = scoutPage.locator('.settings-tab');
    const count = await tabs.count();
    if (count < 3) {
      issue('SETTINGS', `Only ${count} tabs visible, expected at least 3`);
    }

    for (let i = 0; i < count; i++) {
      const label = await tabs.nth(i).textContent({ timeout: 2_000 }).catch(() => '');
      await tabs.nth(i).click();
      await scoutPage.waitForTimeout(500);
      const content = scoutPage.locator('.settings-tab-content, .ps-card');
      if (!await content.first().isVisible({ timeout: 3_000 }).catch(() => false)) {
        issue('SETTINGS', `Tab "${label}" rendered no content`);
      }
    }
  });

  test('404 page', async () => {
    test.skip(!authenticated, 'Registration failed');
    await gotoAuthed('/evm/nonexistent');

    if (!await scoutPage.locator('.evm-not-found').isVisible({ timeout: 5_000 }).catch(() => false)) {
      issue('404', 'Not-found page did not render');
    }
  });

  test('responsive: mobile viewport', async () => {
    test.skip(!authenticated, 'Registration failed');
    await scoutPage.setViewportSize({ width: 375, height: 812 });
    await goto('/evm');

    const hamburger = scoutPage.locator('.mobile-menu-toggle');
    if (!await hamburger.isVisible({ timeout: 5_000 }).catch(() => false)) {
      issue('RESPONSIVE', 'Hamburger not visible on mobile');
    } else {
      await hamburger.click();
      if (!await scoutPage.locator('.sidebar').evaluate(el => el.classList.contains('open')).catch(() => false)) {
        issue('RESPONSIVE', 'Sidebar did not open on hamburger click');
      }
    }

    await scoutPage.setViewportSize({ width: 1280, height: 720 });
  });

  test('cross-component stress', async () => {
    test.skip(!authenticated, 'Registration failed');
    await gotoAuthed('/evm');

    const menu = scoutPage.locator('.user-menu-trigger');
    const store = scoutPage.locator('.store-selector-btn');

    for (let i = 0; i < 5; i++) {
      await menu.click();
      await store.click();
      await menu.click();
      await store.click();
    }

    const btn = scoutPage.locator('.main-header-actions button', { hasText: /create invoice/i });
    const start = Date.now();
    await btn.click();
    const elapsed = Date.now() - start;
    if (elapsed > 500) {
      issue('PERF', `Button response ${elapsed}ms after stress (expected <500ms)`);
    }
    if (!await scoutPage.locator('.modal-overlay').isVisible({ timeout: 2_000 }).catch(() => false)) {
      issue('PERF', 'Modal did not open after stress test');
    }
  });

  // Shared between the two viewport passes below so the crawl - and whatever
  // plugins this server declared - only has to run once.
  let discoveredRoutes: string[] = [];

  test('route coverage: desktop', async () => {
    test.skip(!authenticated, 'Registration failed');
    // A real hard navigation per route, not an SPA transition, so this walk
    // reproduces what a bookmarked link or a reload actually does. That's
    // slower than clicking through the sidebar, hence the wider budget.
    test.setTimeout(90_000);

    // The whole body below is throw-prone (UI flow, a real DB insert) and
    // this describe runs in serial mode, where an uncaught throw doesn't just
    // fail this test - it skips `route coverage: mobile` AND `summary: all
    // issues`, discarding every issue() already collected earlier in the run
    // (a panic, a 500) with nothing to say why. That is the same
    // collected-then-never-asserted failure the ticket exists to close, just
    // moved from "printed instead of failed" to "thrown away by a sibling
    // test's crash." Catching here keeps the rest of the suite - and its
    // verdict - intact; the failure itself still fails the run via issue().
    try {
      // Store, wallet and invoice detail pages need a real record to land on
      // - see the comment above PLACEHOLDER_ROUTES. Creating this invoice
      // also creates the account wallet backing its store's payment method
      // (`store_payment_method.rs`'s `wallet_for_store_xpub`), so one seed
      // covers all three detail pages plus checkout below.
      const seedName = `scout-${Date.now().toString(36)}`;
      await createStoreReadyForInvoices(scoutPage, seedName);
      // createStoreAndOpen (inside createStoreReadyForInvoices) leaves
      // scoutPage on the store's own detail page and nothing between here and
      // the capture navigates away, so this is the store's real id - needed
      // below to check the crawl actually reached this page, not just
      // assumed it would.
      const storeId = new URL(scoutPage.url()).pathname.split('/').pop()!;
      // Shared with seedPaymentForInvoice below - the seeded payment has to
      // pay this exact amount or the invoice stays "underpaid" and the crawl
      // never renders the paid state, which is the one a merchant actually
      // cares about.
      const invoiceAmountEth = '0.01';
      await createInvoice(scoutPage, invoiceAmountEth);
      // createInvoice already waited for the URL to match /evm/invoices/.+,
      // so the last path segment is guaranteed non-empty here.
      const invoiceId = new URL(scoutPage.url()).pathname.split('/').pop()!;
      // See seedPaymentForInvoice's comment in fixtures/db.ts: a real payment
      // needs on-chain settlement this walk can't produce, so this inserts
      // the row directly - only possible where a DB connection exists, which
      // E2E_REMOTE's live-deployment runs do not have.
      const paymentId = isRemote
        ? undefined
        : await seedPaymentForInvoice(invoiceId, parseEther(invoiceAmountEth));

      await gotoAuthed('/evm');
      // '/evm' itself has to be seeded explicitly: discoverLinkedRoutes only
      // returns links found ON this page, not the path of the page itself,
      // and nothing guarantees the dashboard renders a self-referential <a
      // href="/evm">. Without this, the busiest page in the app - the one
      // every session lands on - would only reach the mobile pass by
      // accident.
      //
      // checkout_url() in the client's invoice detail page renders the
      // customer-facing link as an absolute URL behind a "Copy link" button,
      // not an <a href>, so it can never turn up in discoverLinkedRoutes -
      // reaching it needs the real id captured above.
      const dashboardLinks = await discoverLinkedRoutes();
      if (dashboardLinks.length < MIN_DASHBOARD_LINKS) {
        issue(
          'ROUTE_DISCOVERY',
          `Dashboard sidebar rendered only ${dashboardLinks.length} route link(s), expected at least ${MIN_DASHBOARD_LINKS} - a route may have silently dropped out of the nav`,
        );
      }

      discoveredRoutes = await walkRoutes([
        '/evm',
        ...dashboardLinks,
        ...(await discoverPluginRoutes()),
        `/checkout/${invoiceId}`,
        ...PLACEHOLDER_ROUTES,
      ]);

      // The comment above PLACEHOLDER_ROUTES assumes the store/invoice list
      // pages render a plain <a href> to the record just seeded, so the
      // crawl finds it on its own - never actually checked. If either list
      // renders its row via a click handler or a non-anchor element instead,
      // the crawl silently never visits that detail page and the walk above
      // still "succeeds" having covered neither. `discoveredRoutes` is every
      // path the walk actually landed on, so checking it here is checking
      // the real outcome instead of the assumption.
      if (!discoveredRoutes.includes(`/evm/stores/${storeId}`)) {
        issue('ROUTE_DISCOVERY', `Store detail page /evm/stores/${storeId} was never crawled - the store card's link may not be a plain <a href>`);
      }
      if (!discoveredRoutes.includes(`/evm/invoices/${invoiceId}`)) {
        issue('ROUTE_DISCOVERY', `Invoice detail page /evm/invoices/${invoiceId} was never crawled - the invoice row's link may not be a plain <a href>`);
      }
      if (paymentId) {
        if (!discoveredRoutes.includes(`/evm/payments/${paymentId}`)) {
          issue('ROUTE_DISCOVERY', `Payment detail page /evm/payments/${paymentId} was never crawled - the payment row's link may not be a plain <a href>`);
        }
        // Reaching the invoice detail URL proves nothing about what it
        // showed - seedPaymentForInvoice exists specifically to make this
        // invoice render as paid, and a page stuck on "Underpaid" (a
        // parseEther-vs-server decimal drift, or paid-status derived from a
        // field this raw INSERT doesn't populate) would still pass the
        // route-reachability checks above. Reading the rendered status and
        // amount is what actually exercises the path "the one that matters
        // most to a merchant" - route coverage alone throws that signal away.
        await gotoAuthed(`/evm/invoices/${invoiceId}`);
        const statusText = await scoutPage
          .locator('.invoice-detail-title-row .badge')
          .textContent({ timeout: 5_000 })
          .catch(() => null);
        if (statusText?.trim() !== 'Paid') {
          issue('PAYMENT', `Invoice ${invoiceId} shows status "${statusText?.trim() ?? '(not found)'}" after a full payment was seeded - expected "Paid"`);
        }
        // Scoped to the "Amount received" row specifically, not just the
        // success-styled class - a second green-styled value elsewhere on
        // the card (a confirmation count, a fee line) would otherwise let
        // `.first()` silently read the wrong field.
        const receivedText = await scoutPage
          .locator('.detail-row', { hasText: 'Amount received' })
          .locator('.detail-value-success')
          .first()
          .textContent({ timeout: 5_000 })
          .catch(() => null);
        // A substring check here (`receivedText.includes(invoiceAmountEth)`)
        // would pass on "10.01" or "0.010" ETH just as readily as on "0.01" -
        // the exact wrong-decimal bug this assertion exists to catch. Extract
        // the numeric token and compare it as a number instead.
        const receivedAmount = Number(receivedText?.match(/-?\d+(\.\d+)?/)?.[0]);
        if (receivedAmount !== Number(invoiceAmountEth)) {
          issue('PAYMENT', `Invoice ${invoiceId} shows amount received "${receivedText?.trim() ?? '(not found)'}" - expected ${invoiceAmountEth}`);
        }
      } else {
        // isRemote: no DB to seed a payment against, so there is no
        // paymentId to check and the branch above silently never runs.
        // Recorded so the gap is visible in every remote run's issue log
        // rather than looking like a check that passed - see gatingIssues()
        // for why this doesn't fail the run.
        issue('COVERAGE_GAP', 'Payment detail page and paid status were not checked - E2E_REMOTE has no DB to seed a payment against');
      }
      // No id captured for the wallet the invoice's payment method implicitly
      // created (see the comment above), so this checks the shape of the
      // route rather than a specific one - a fresh scout account has exactly
      // one.
      if (!discoveredRoutes.some((route) => /^\/evm\/wallets\/[^/]+$/.test(route))) {
        issue('ROUTE_DISCOVERY', 'No wallet detail page was crawled - the wallet card\'s link may not be a plain <a href>');
      }
    } catch (err) {
      // gotoAuthed (called above, both for '/evm' and for the invoice detail
      // page) throws its own SkipError via test.skip() when the session was
      // lost - already recorded as a SESSION issue and meant to end this
      // test as "skipped", not "failed". Swallowing that here instead of
      // re-throwing would fight gotoAuthed's own doc comment, which exists
      // specifically to make a lost session a clean skip rather than a
      // cascading failure. Anything else - a UI flow that stalled, the raw
      // DB insert - really is unhandled, and becomes a SETUP issue instead
      // of taking the rest of this serial file down with it.
      //
      // test.skip() sets `expectedStatus`, not `status` - `status` is only
      // populated once the test has actually finished (see TestInfo's own
      // docs), so it reads as undefined here, mid-test, no matter what threw.
      // Checking it would make this branch dead code and swallow every
      // SkipError into a SETUP issue instead of letting it skip.
      if (test.info().expectedStatus === 'skipped') {
        throw err;
      }
      issue('SETUP', `route coverage: desktop failed: ${err instanceof Error ? err.message : String(err)}`);
    }
  });

  test('route coverage: mobile', async () => {
    test.skip(!authenticated, 'Registration failed');
    test.skip(discoveredRoutes.length === 0, 'Desktop route coverage did not run');
    test.setTimeout(90_000);

    // Every UI bug reported by hand recently was on a phone - a table
    // overflowing its card, an unlabelled control, an amount clipped
    // mid-number - and nothing here had ever loaded a single page at a phone
    // width to catch that class of problem before a person did. The overflow
    // check below catches the first of those directly; an unlabelled control
    // or a clipped number has no cheap, reliable DOM signal the way a
    // horizontal overflow does; visible-but-wrong is a screenshot-diffing
    // problem, not "nearly free", so it stays out of this pass rather than
    // becoming a check that always passes.
    // Same reasoning as `route coverage: desktop` above: this walk calls
    // gotoAuthed (via walkRoutes) and hits the same throw-prone
    // checkout cookie/localStorage dance, so an uncaught throw here would
    // just as readily skip `summary: all issues` and discard every issue()
    // collected across the whole run. The desktop test was hardened against
    // this; this one wasn't, and it's at least as throw-prone since it's the
    // pass that exists specifically to hit newly-flaky mobile rendering.
    await scoutPage.setViewportSize(MOBILE_VIEWPORT);
    try {
      await walkRoutes(discoveredRoutes, { crawl: false, checkOverflow: true });
    } catch (err) {
      // See the matching catch in `route coverage: desktop` for why a lost
      // session (SkipError, expectedStatus === 'skipped') re-throws instead
      // of being recorded as a SETUP issue.
      if (test.info().expectedStatus === 'skipped') {
        throw err;
      }
      issue('SETUP', `route coverage: mobile failed: ${err instanceof Error ? err.message : String(err)}`);
    } finally {
      await scoutPage.setViewportSize(DESKTOP_VIEWPORT);
    }
  });
});

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

test('summary: all issues', async () => {
  const significant = consoleErrors.filter(
    e => !e.includes('favicon') && !e.includes('ResizeObserver'),
  );

  if (significant.length > 0) {
    console.log(`\n🔴 ${significant.length} JS errors:`);
    for (const e of significant) console.log(`    ${e}`);
  }

  if (issues.length > 0) {
    console.log(`\n🔴 ${issues.length} issues:`);
    for (const i of issues) console.log(`    ${i}`);
  }

  // Only fail on gating issues - see gatingIssues() for which labels are
  // exempted and why. scout-filter.spec.ts tests this filter directly.
  const gating = gatingIssues(issues);
  expect(gating, `Gating issues:\n${gating.join('\n')}`).toHaveLength(0);
});
