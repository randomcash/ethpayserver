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
  // log had not one line of Playwright output. `github` adds failure
  // annotations on the diff.
  reporter: process.env.CI
    ? [['list'], ['github'], ['html', { open: 'never' }], ['./perf-reporter.ts']]
    : [['list'], ['./perf-reporter.ts']],
  timeout: REMOTE ? 60_000 : 30_000,
  use: {
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
