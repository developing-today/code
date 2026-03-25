import { defineConfig, devices } from "@playwright/test";

/**
 * Playwright E2E test configuration for the id web UI.
 *
 * Runs tests against both Chromium and Firefox.
 * In Nix environments, browser executable paths are set via env vars.
 *
 * Usage:
 *   cd e2e && bun install && bunx playwright test
 *   just test-e2e              # runs against both browsers
 *   just test-e2e-chromium     # chromium only
 *   just test-e2e-firefox      # firefox only
 */

const PORT = Number(process.env.TEST_PORT) || 4173;

export default defineConfig({
  testDir: "./tests",
  fullyParallel: false, // tests share server state
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: 1, // single server instance
  reporter: [["html", { open: "never" }], ["list"]],
  timeout: 30_000,

  use: {
    baseURL: `http://localhost:${PORT}`,
    trace: "on-first-retry",
    screenshot: "only-on-failure",
  },

  projects: [
    {
      name: "chromium",
      use: {
        ...devices["Desktop Chrome"],
        launchOptions: {
          executablePath:
            process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH || undefined,
        },
      },
    },
    {
      name: "firefox",
      use: {
        ...devices["Desktop Firefox"],
        launchOptions: {
          executablePath:
            process.env.PLAYWRIGHT_FIREFOX_EXECUTABLE_PATH || undefined,
        },
      },
    },
  ],

  webServer: {
    command: `../target/debug/id serve --web --port ${PORT} --ephemeral`,
    port: PORT,
    reuseExistingServer: !process.env.CI,
    timeout: 60_000, // allow time for server startup
    stdout: "pipe",
    stderr: "pipe",
  },
});
