import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

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
//
// NOTE (#229): that isolates browsers, not spec files — every spec in a
// project shares that project's server (and its in-memory store) for the
// whole invocation. Runners therefore invoke Playwright once per spec
// file so each file gets a fresh ephemeral server: see the test-e2e
// check in ../flake.nix and the test-e2e* recipes in ../justfile. A
// single full-suite invocation (e.g. `bunx playwright test` with no file
// argument) does NOT have per-spec isolation.
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

// World E2E (#1322): the world-surface spec joins the lobby world, so the
// test servers host one with the roc-screen fixture program (a Screen-surface
// counter: "+" keypresses and pastes increment it, layout reflows 30<->20
// columns). The fixture is checked in under e2e/fixtures so the spec needs
// no Roc toolchain; refresh it from examples/roc-screen after a platform ABI
// change. The token must match the spec default (and the NixOS VM wiring in
// nix/tests/playwright-e2e-test.nix); override with ID_E2E_WORLD_ADMIN_TOKEN
// in all three places at once.
const WORLD_ADMIN_TOKEN = process.env.ID_E2E_WORLD_ADMIN_TOKEN ?? "e2e-world-admin-token";
// Absolute path: `id --data-dir` chdirs the server into its data dir, so a
// cwd-relative module path would no longer resolve to the checked-in fixture.
const WORLD_MODULE = path.join(path.dirname(fileURLToPath(import.meta.url)), "fixtures/roc-screen.wasm");
const WORLD_FLAGS = `--world --world-admin-token ${WORLD_ADMIN_TOKEN} --world-module ${WORLD_MODULE}`;

// Each server gets its own data dir (#15145): the two config-managed servers
// used to start with identical flags from the same working directory, so
// they shared .iroh-key (same iroh endpoint id) and the on-disk event store
// — the second server exited immediately with "events.redb is open in
// another process" and local mode could not start at all. `--data-dir`
// chdirs the server into the given directory, isolating its key, event
// store, identity db and serve lock. A fresh temp root per config load
// keeps the per-invocation "fresh ephemeral server" model intact: no state
// leaks between runs, and parallel invocations never collide.
const DATA_ROOT = mkdtempSync(path.join(tmpdir(), "id-e2e-data-"));
const CHROMIUM_DATA_DIR = path.join(DATA_ROOT, `chromium-${CHROMIUM_PORT}`);
const FIREFOX_DATA_DIR = path.join(DATA_ROOT, `firefox-${FIREFOX_PORT}`);

const chromiumServer = {
  command: `../target/debug/id --data-dir ${CHROMIUM_DATA_DIR} serve --web --port ${CHROMIUM_PORT} --ephemeral${OFFLINE_FLAGS} ${WORLD_FLAGS}`,
  port: CHROMIUM_PORT,
  reuseExistingServer: !process.env.CI && !IS_NIX_BUILD,
  timeout: 60_000,
  stdout: "pipe" as const,
  stderr: "pipe" as const,
};

const firefoxServer = {
  command: `../target/debug/id --data-dir ${FIREFOX_DATA_DIR} serve --web --port ${FIREFOX_PORT} --ephemeral${OFFLINE_FLAGS} ${WORLD_FLAGS}`,
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
