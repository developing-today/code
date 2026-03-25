import { defineConfig, devices } from "@playwright/test";

/**
 * Playwright E2E test configuration for the id web UI.
 *
 * Runs tests against both Chromium and Firefox.
 * In Nix environments, set PLAYWRIGHT_BROWSERS_PATH to the nix-provided
 * playwright-driver.browsers output (done automatically in nix-common.nix).
 *
 * Usage:
 *   cd e2e && bun install && bunx playwright test
 *   just test-e2e              # runs against both browsers
 *   just test-e2e-chromium     # chromium only
 *   just test-e2e-firefox      # firefox only
 */

// Each browser project gets its own ephemeral server on a different port
// to avoid shared state issues (e.g., files created in chromium tests
// appearing in firefox tests).
const CHROMIUM_PORT = Number(process.env.TEST_PORT) || 4173;
const FIREFOX_PORT = CHROMIUM_PORT + 1;

export default defineConfig({
  testDir: "./tests",
  fullyParallel: false, // tests within a project run sequentially
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: 1,
  reporter: [["html", { open: "never" }], ["list"]],
  timeout: 30_000,

  use: {
    trace: "on-first-retry",
    screenshot: "only-on-failure",
  },

  projects: [
    {
      name: "chromium",
      use: {
        ...devices["Desktop Chrome"],
        baseURL: `http://localhost:${CHROMIUM_PORT}`,
      },
    },
    {
      name: "firefox",
      use: {
        ...devices["Desktop Firefox"],
        baseURL: `http://localhost:${FIREFOX_PORT}`,
      },
    },
  ],

  webServer: [
    {
      command: `../target/debug/id serve --web --port ${CHROMIUM_PORT} --ephemeral`,
      port: CHROMIUM_PORT,
      reuseExistingServer: !process.env.CI,
      timeout: 60_000,
      stdout: "pipe",
      stderr: "pipe",
    },
    {
      command: `../target/debug/id serve --web --port ${FIREFOX_PORT} --ephemeral`,
      port: FIREFOX_PORT,
      reuseExistingServer: !process.env.CI,
      timeout: 60_000,
      stdout: "pipe",
      stderr: "pipe",
    },
  ],
});
