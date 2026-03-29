import { defineConfig, devices } from "@playwright/test";

/**
 * Playwright E2E test configuration for the id web UI.
 *
 * Three execution modes:
 *
 * 1. LOCAL (default): Both Chromium and Firefox, servers started automatically.
 *      cd e2e && bun install && bunx playwright test
 *      just test-e2e
 *
 * 2. NIX SANDBOX (NIX_BUILD_TOP set): Firefox only — Chromium crashes in the
 *    nix build sandbox due to kernel-level restrictions on namespaces/seccomp.
 *      nix build .#checks.x86_64-linux.test-e2e
 *
 * 3. VM TEST (PLAYWRIGHT_VM_TEST=1): Both browsers, servers on separate VMs.
 *    No local webServer — tests connect to remote id services via base URL
 *    env vars (CHROMIUM_BASE_URL, FIREFOX_BASE_URL). Full Playwright suite
 *    runs inside NixOS VMs where Chromium works (no sandbox restrictions).
 *      nix build .#checks.x86_64-linux.nixos-playwright-e2e
 *
 * In Nix environments, set PLAYWRIGHT_BROWSERS_PATH to the nix-provided
 * playwright-driver.browsers output (done automatically in nix-common.nix).
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

// VM test mode: servers run on separate NixOS VMs (4-VM architecture).
// No local webServer — tests connect to remote id services via base URL env vars.
const IS_VM_TEST = !!process.env.PLAYWRIGHT_VM_TEST;

// Allow overriding base URLs for VM/remote testing where servers
// run on different hosts than the test runner.
const CHROMIUM_BASE = process.env.CHROMIUM_BASE_URL || `http://localhost:${CHROMIUM_PORT}`;
const FIREFOX_BASE = process.env.FIREFOX_BASE_URL || `http://localhost:${FIREFOX_PORT}`;

// Chromium needs extra flags when using nix-provided browsers — both in nix
// sandbox builds AND NixOS VM tests. The nix-patched Chromium binary runs
// as root in VMs and needs --no-sandbox, plus multi-process flags to avoid
// crashes from missing /dev/shm or GPU access.
const CHROMIUM_NIX_ARGS =
  IS_NIX_BUILD || IS_VM_TEST
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
    baseURL: CHROMIUM_BASE,
    ...(CHROMIUM_NIX_ARGS.length > 0 && {
      launchOptions: { args: CHROMIUM_NIX_ARGS },
    }),
  },
};

const firefoxProject = {
  name: "firefox",
  use: {
    ...devices["Desktop Firefox"],
    baseURL: FIREFOX_BASE,
  },
};

// In nix sandbox: Firefox only (Chromium crashes in sandbox).
// In VM test or local: both browsers (each VM runs one browser via --project).
const projects = IS_NIX_BUILD && !IS_VM_TEST ? [firefoxProject] : [chromiumProject, firefoxProject];

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

// In VM test: no webServer (servers run on separate VMs).
// In nix sandbox: Firefox server only.
// Local: both servers.
const webServers = IS_VM_TEST ? [] : IS_NIX_BUILD ? [firefoxServer] : [chromiumServer, firefoxServer];

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
