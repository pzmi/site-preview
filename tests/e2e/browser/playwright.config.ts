// Playwright configuration for the browser tests (E2E-04, ADR 0016).
//
// Chromium comes from the environment: `PLAYWRIGHT_BROWSERS_PATH` points at a preinstalled
// build (the agent sandbox uses /opt/pw-browsers). Nothing here downloads a browser; `cargo
// xtask test --only browser` fails with an install hint when none is present.
import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: '.',
  testMatch: '**/*.spec.ts',
  // Failure evidence (traces, screenshots, attachments) lands here; it is gitignored.
  outputDir: 'test-results',
  // Every test starts its own server on its own port in its own tempdir, so tests are
  // independent and may run in parallel.
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  // Generous: a bound on how long a failing test hangs, never the oracle (plan section 12).
  timeout: 60_000,
  expect: { timeout: 15_000 },
  reporter: [['list']],
  use: {
    browserName: 'chromium',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
});
