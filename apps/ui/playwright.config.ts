import { defineConfig, devices } from '@playwright/test';

export const E2E_SUITE_FILES = {
  required: [
    'navigation.spec.ts',
    'persistent-surfaces.spec.ts',
    'smoke.spec.ts',
  ],
  'live-daemon': [
    'dashboard.spec.ts',
    'mission-control.spec.ts',
    'scoped-subscription.spec.ts',
    'session-messages.spec.ts',
  ],
  'provider-backed': [
    'long-running/multi-turn.spec.ts',
    'long-running/research.spec.ts',
    'quick-chat.spec.ts',
    'session-debug.spec.ts',
  ],
  diagnostic: [
    'debug-click-session.spec.ts',
    'debug-issues.spec.ts',
    'subscription-debug.spec.ts',
  ],
} as const;

/**
 * Playwright configuration for AgentZero E2E tests.
 * @see https://playwright.dev/docs/test-configuration
 */
export default defineConfig({
  testDir: './tests/e2e',
  
  // Run tests in parallel
  fullyParallel: true,
  
  // Fail the build on CI if you accidentally left test.only in the source code
  forbidOnly: !!process.env.CI,
  
  // Retry on CI only
  retries: process.env.CI ? 2 : 0,
  
  // Opt out of parallel tests on CI
  workers: process.env.CI ? 1 : undefined,
  
  // Reporter to use
  reporter: [
    ['html', { outputFolder: 'playwright-report' }],
    ['list'],
  ],
  
  // Shared settings for all the projects below
  use: {
    // Base URL to use in actions like `await page.goto('/')`
    baseURL: 'http://localhost:3000',
    
    // Collect trace when retrying the failed test
    trace: 'on-first-retry',
    
    // Capture screenshot on failure
    screenshot: 'only-on-failure',
    
    // Record video on failure
    video: 'on-first-retry',
  },

  // Each spec belongs to exactly one prerequisite-based execution lane.
  projects: [
    {
      name: 'required',
      testMatch: [...E2E_SUITE_FILES.required],
      use: { ...devices['Desktop Chrome'] },
    },
    {
      name: 'live-daemon',
      testMatch: [...E2E_SUITE_FILES['live-daemon']],
      use: { ...devices['Desktop Chrome'] },
    },
    {
      name: 'provider-backed',
      testMatch: [...E2E_SUITE_FILES['provider-backed']],
      use: { ...devices['Desktop Chrome'] },
    },
    {
      name: 'diagnostic',
      testMatch: [...E2E_SUITE_FILES.diagnostic],
      use: { ...devices['Desktop Chrome'] },
    },
  ],

  // Run local dev server before starting the tests
  webServer: {
    command: 'npm run dev',
    url: 'http://localhost:3000',
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
  
  // Global timeout for each test
  timeout: 60_000,
  
  // Timeout for expect assertions
  expect: {
    timeout: 10_000,
  },
});
