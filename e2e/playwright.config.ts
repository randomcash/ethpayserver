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
  reporter: process.env.CI
    ? [['html', { open: 'never' }], ['./perf-reporter.ts']]
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
