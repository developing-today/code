import { defineConfig, devices } from "@playwright/test";

/**
 * Playwright E2E test configuration for the id web UI.
 *
 * Locally: runs tests against both Chromium and Firefox.
 * In nix sandbox: runs Firefox only (Chromium has shared-lib issues in the
 * sandboxed build environment).
 *
 * In Nix environments, set PLAYWRIGHT_BROWSERS_PATH to the nix-provided
 * playwright-driver.browsers output (done automatically in nix-common.nix).
 *
 * Usage:
 *   cd e2e && bun install && bunx playwright test
 *   just test-e2e              # runs against both browsers (or firefox-only in nix)
 *   just test-e2e-chromium     # chromium only
 *   just test-e2e-firefox      # firefox only
 */

// Each browser project gets its own ephemeral server on a different port
// to avoid shared state issues (e.g., files created in chromium tests
// appearing in firefox tests).
const CHROMIUM_PORT = Number(process.env.TEST_PORT) || 4173;
const FIREFOX_PORT = CHROMIUM_PORT + 1;

// In nix sandbox builds, disable P2P networking (no UDP sockets available).
// E2E tests only exercise the web UI, not P2P features.
const IS_NIX_BUILD = !!process.env.NIX_BUILD_TOP;
const OFFLINE_FLAGS = IS_NIX_BUILD ? " --no-mdns --no-relay --no-gossip" : "";

// Chromium fails in nix sandbox due to missing shared libraries / /dev/shm.
// Firefox works reliably, so we skip Chromium in sandboxed nix builds.
const projects = IS_NIX_BUILD
  ? [
      {
        name: "firefox",
        use: {
          ...devices["Desktop Firefox"],
          baseURL: `http://localhost:${FIREFOX_PORT}`,
        },
      },
    ]
  : [
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
    ];

const webServers = IS_NIX_BUILD
  ? [
      {
        command: `../target/debug/id serve --web --port ${FIREFOX_PORT} --ephemeral${OFFLINE_FLAGS}`,
        port: FIREFOX_PORT,
        reuseExistingServer: false,
        timeout: 60_000,
        stdout: "pipe" as const,
        stderr: "pipe" as const,
      },
    ]
  : [
      {
        command: `../target/debug/id serve --web --port ${CHROMIUM_PORT} --ephemeral${OFFLINE_FLAGS}`,
        port: CHROMIUM_PORT,
        reuseExistingServer: !process.env.CI,
        timeout: 60_000,
        stdout: "pipe" as const,
        stderr: "pipe" as const,
      },
      {
        command: `../target/debug/id serve --web --port ${FIREFOX_PORT} --ephemeral${OFFLINE_FLAGS}`,
        port: FIREFOX_PORT,
        reuseExistingServer: !process.env.CI,
        timeout: 60_000,
        stdout: "pipe" as const,
        stderr: "pipe" as const,
      },
    ];

export default defineConfig({
  testDir: "./tests",
  fullyParallel: false, // tests within a project run sequentially
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 1,
  workers: 1,
  reporter: [["html", { open: "never" }], ["list"]],
  timeout: 30_000,

  use: {
    trace: "on-first-retry",
    screenshot: "only-on-failure",
  },

  projects,
  webServer: webServers,
});
