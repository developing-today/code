import { defineConfig, devices } from "@playwright/test";

/**
 * Playwright E2E test configuration for the id web UI.
 *
 * Locally: runs tests against both Chromium and Firefox.
 * In nix sandbox: runs Firefox only — Chromium's multi-process architecture
 * crashes in the nix build sandbox (kernel-level restrictions on namespaces,
 * process management, and /proc access that --no-sandbox can't work around).
 * Chromium is separately tested via the `nixos-e2e` NixOS VM check.
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

// Chromium in nix sandbox crashes due to kernel-level restrictions on namespaces,
// process management, and /proc access that no combination of --no-sandbox,
// --no-zygote, --disable-gpu, etc. can work around. Run Firefox-only in nix;
// Chromium is covered by the `nixos-e2e` NixOS VM integration test.
const CHROMIUM_NIX_ARGS = IS_NIX_BUILD
  ? [
      "--no-sandbox",
      "--disable-setuid-sandbox",
      "--no-zygote",
      "--disable-dev-shm-usage",
      "--disable-gpu",
      "--disable-software-rasterizer",
    ]
  : [];

const chromiumProject = {
  name: "chromium",
  use: {
    ...devices["Desktop Chrome"],
    baseURL: `http://localhost:${CHROMIUM_PORT}`,
    ...(CHROMIUM_NIX_ARGS.length > 0 && {
      launchOptions: { args: CHROMIUM_NIX_ARGS },
    }),
  },
};

const firefoxProject = {
  name: "firefox",
  use: {
    ...devices["Desktop Firefox"],
    baseURL: `http://localhost:${FIREFOX_PORT}`,
  },
};

// In nix sandbox: Firefox only (Chromium crashes).
// Elsewhere: both browsers.
const projects = IS_NIX_BUILD ? [firefoxProject] : [chromiumProject, firefoxProject];

const chromiumServer = {
  command: `../target/debug/id serve --web --port ${CHROMIUM_PORT} --ephemeral${OFFLINE_FLAGS}`,
  port: CHROMIUM_PORT,
  reuseExistingServer: !process.env.CI && !IS_NIX_BUILD,
  timeout: 60_000,
  stdout: "pipe" as const,
  stderr: "pipe" as const,
};

const firefoxServer = {
  command: `../target/debug/id serve --web --port ${FIREFOX_PORT} --ephemeral${OFFLINE_FLAGS}`,
  port: FIREFOX_PORT,
  reuseExistingServer: !process.env.CI && !IS_NIX_BUILD,
  timeout: 60_000,
  stdout: "pipe" as const,
  stderr: "pipe" as const,
};

// Only start servers for enabled browser projects.
const webServers = IS_NIX_BUILD ? [firefoxServer] : [chromiumServer, firefoxServer];

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
