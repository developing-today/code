import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawn, type Subprocess } from "bun";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

// #1267: /api/health must not lie by request order. The live source
// populates provenance strings lazily — historically, a fresh process
// whose FIRST request was /api/health reported "fixtures" for facets
// that were really live, because no page visit had exercised them yet.
// This test boots a live-source server against a controlled feed and
// asserts on the FIRST response the process ever serves.

interface HealthBody {
  status: string;
  source: Record<string, string>;
  sections?: { orchestration: { state: string; runId?: string } };
}

let proc: Subprocess | undefined;
let firstHealth: HealthBody | undefined;

beforeAll(async () => {
  const dir = await mkdtemp(join(tmpdir(), "pm-health-"));
  const now = new Date().toISOString();
  const feed = {
    version: 1,
    generatedAt: now,
    quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
    attention: [],
    orchestration: {
      version: 1,
      generatedAt: now,
      runId: "orch-health-test",
      provenance: { sources: ["health priming test"], runId: "orch-health-test" },
      tiers: { needsDrew: [], needsOrchestrator: [] },
      dependencies: [],
      goals: [],
      actions: [],
    },
    registry: [{ id: "ses-health-1", source: "opencode-sandbox", label: "Health probe session" }],
  };
  const feedFile = join(dir, "fleet-feed.json");
  await writeFile(feedFile, JSON.stringify(feed));
  const port = 5100 + Math.floor(Math.random() * 800);
  proc = spawn({
    cmd: ["bun", "run", "src/server.ts"],
    cwd: join(import.meta.dir, ".."),
    env: {
      ...Bun.env,
      PM_AGG_PORT: String(port),
      PM_AGG_SOURCE: "live",
      PM_AGG_FEED_FILE: feedFile,
      PM_AGG_REGISTRY_FILE: join(dir, "registry.json"),
      PM_AGG_COMMENTS_FILE: join(dir, "comments.json"),
      PM_AGG_SNAPSHOTS_DIR: join(dir, "snapshots"),
      PM_AGG_OPENCHAMBER_STATE: join(dir, "no-state.json"),
      PM_AGG_QUOTA_VIEW: join(dir, "no-quota-view.md"),
      PM_AGG_QUOTA_REGISTRY: join(dir, "no-quota-registry.md"),
      PM_PROJECTOR_URL: "off",
      KANEO_ADMIN_EMAIL: "",
      KANEO_ADMIN_PASSWORD: "",
      GRIST_API_KEY: "",
    },
    stdout: "pipe",
    stderr: "inherit",
  });
  // Poll until the server answers; the first successful body IS the
  // assertion target — nothing else touches this process beforehand.
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline && !firstHealth) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/api/health`);
      if (res.ok) firstHealth = (await res.json()) as HealthBody;
    } catch {
      /* not up yet */
    }
    if (!firstHealth) await Bun.sleep(150);
  }
  expect(firstHealth).toBeDefined();
});

afterAll(() => {
  proc?.kill();
});

describe("/api/health provenance priming (#1267)", () => {
  test("the first-ever response reports the live facets truthfully", () => {
    expect(firstHealth?.status).toBe("ok");
    expect(firstHealth?.source.quota).toContain("live (fleet feed");
    expect(firstHealth?.source.orchestration).toContain("live (orchestration section orch-health-test");
    expect(firstHealth?.source.registry).toContain("1 session records");
    expect(firstHealth?.source.registry).not.toBe("fixtures");
    expect(firstHealth?.sections?.orchestration.state).toBe("live");
    expect(firstHealth?.sections?.orchestration.runId).toBe("orch-health-test");
  });
});
