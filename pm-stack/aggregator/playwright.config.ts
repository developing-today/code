// MOB-00 (#9467): deterministic browser acceptance harness for the
// pm-stack aggregator's mobile work views. The harness boots the real
// server in fixture mode (PM_AGG_SOURCE=fixtures — the same fixture
// contract as the bun suite) on loopback-only ports and drives it with
// a pinned @playwright/test + Chromium. No GitHub/Project/session
// credentials are involved; the spec additionally aborts every request
// that is not aimed at the fixture server, so a passing run proves the
// flow works on isolated deterministic fixture data only.
//
// Trace and video are retained on failure (never a screenshot-only
// pass). The bun contract/read-model fixtures stay in `bun test` and
// are not part of this config: testDir is test/browser only.
import { defineConfig } from "@playwright/test";

const PORT = 4797;
const FRESHNESS_PORT = 4897;
const ORIGIN = `http://127.0.0.1:${PORT}`;

export default defineConfig({
  testDir: "./test/browser",
  timeout: 60_000,
  expect: { timeout: 10_000 },
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [["list"]],
  use: {
    baseURL: ORIGIN,
    viewport: { width: 360, height: 740 },
    trace: "retain-on-failure",
    video: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [{ name: "chromium-360", use: { browserName: "chromium" } }],
  webServer: {
    command: "bun run src/server.ts",
    url: `${ORIGIN}/api/health`,
    reuseExistingServer: false,
    timeout: 60_000,
    env: {
      PM_AGG_SOURCE: "fixtures",
      PM_AGG_PORT: String(PORT),
      PM_AGG_FRESHNESS_PORT: String(FRESHNESS_PORT),
    },
  },
});
