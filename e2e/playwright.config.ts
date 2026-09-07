import { defineConfig } from '@playwright/test';

// `=== 'true'`, not truthiness: `E2E_REMOTE=false` would otherwise select the
// remote origin. Must stay in step with fixtures/api.ts and fixtures/db.ts.
const REMOTE = process.env.E2E_REMOTE === 'true';

const API_URL = process.env.E2E_API_URL || (REMOTE ? 'https://testnet.random.cash' : 'http://localhost:3000');
const BASE_URL = process.env.E2E_BASE_URL || (REMOTE ? 'https://testnet.random.cash' : 'http://localhost:8080');

export default defineConfig({
  testDir: './tests',
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: 1,
  // `list` first in CI too. The html reporter writes a file and prints nothing
  // until the run ends, so a job killed at its timeout-minutes cap produced 30
  // minutes of total silence and no way to tell which test was hanging - the
  // log had not one line of Playwright output.
  //
  // `list` alone still would not name the hanging test: onTestBegin returns
  // early when the output is not a TTY, and an Actions log is not one, so only
  // finished tests print. The workflow sets PLAYWRIGHT_LIST_PRINT_STEPS=1,
  // which makes onStepEnd print regardless of TTY - progress from *inside* the
  // running test, which is what a hang needs. `line` is not the answer here: it
  // emits cursor-control escapes that render as garbage in a non-TTY log.
  //
  // Deliberately no `github` reporter. The E2E job is skipped on pull_request
  // (it is gated on refs/heads/*), so annotations could never reach a PR diff,
  // and on testnet pushes the suite is knowingly red under continue-on-error
  // (RCS-192) - it would stamp ~56 error annotations on every push for failures
  // already tracked. It belongs in the commit that removes continue-on-error.
  reporter: process.env.CI
    ? [['list'], ['html', { open: 'never' }], ['./perf-reporter.ts']]
    : [['list'], ['./perf-reporter.ts']],
  timeout: REMOTE ? 60_000 : 30_000,
  use: {
    // Bound every locator action. Playwright's default is no timeout at all, so
    // a click on an element that is missing - or, as in the invoice modal, one
    // that stays disabled - waits until the TEST times out. The failure then
    // names the test rather than the action, and in a serial file it takes the
    // rest of the file down with it. That pattern cost this suite the whole
    // 30-minute job cap more than once.
    //
    // 15s is far above any legitimate action here (registration measures 3.6s
    // locally, 5.3s at 6x CPU throttle) and well under the 30s test budget, so
    // a stuck action fails as itself with its own call log. Anything genuinely
    // slower passes an explicit timeout. Note this covers actions only:
    // waitFor, waitForURL and expect() carry their own timeouts.
    actionTimeout: 15_000,
    baseURL: BASE_URL,
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
  },
  projects: [
    {
      name: 'chromium',
      use: { browserName: 'chromium' },
    },
  ],
  ...(!REMOTE && {
    webServer: [
      {
        command: 'cargo run --release --bin ethpayserver',
        url: `${API_URL}/health/live`,
        reuseExistingServer: true,
        timeout: 120_000,
      },
      {
        command: 'trunk serve',
        cwd: '../client',
        url: BASE_URL,
        reuseExistingServer: true,
        timeout: 60_000,
      },
    ],
  }),
});
