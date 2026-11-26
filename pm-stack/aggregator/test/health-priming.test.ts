import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawn, type Subprocess } from "bun";
import { mkdtemp, readdir, writeFile } from "node:fs/promises";
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
  degraded?: boolean;
  source: Record<string, string>;
  sections?: { orchestration: { state: string; runId?: string } };
}

let proc: Subprocess | undefined;
let firstHealth: HealthBody | undefined;
let packageDirBefore: string[] = [];
const packageDir = join(import.meta.dir, "..");

// #10467: starting the aggregator subprocess can take longer than bun's
// default 5 s hook budget on a loaded machine; give the hook room to
// reach its own readiness deadline and fail with that message instead.
beforeAll(async () => {
  const dir = await mkdtemp(join(tmpdir(), "pm-health-"));
  const now = new Date().toISOString();
  const feed = {
    version: 1,
    generatedAt: now,
    quota: {
      pools: [
        {
          id: "quota-health-1",
          provider: "openai",
          plan: "test",
          state: "healthy",
          windows: [],
          credentials: [{ label: "malformed credential", providerId: "openai", windows: [] }],
          activeSessions: [null],
        },
      ],
      burn: [],
      choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] },
    },
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
    registry: [
      {
        id: "ses-health-1",
        source: "opencode-sandbox",
        label: "Health probe session",
        status: "idle",
        agentId: "opencode:main",
        agentLabel: "OpenCode · main (sandbox)",
        taskRefs: [],
      },
    ],
  };
  const feedFile = join(dir, "fleet-feed.json");
  await writeFile(feedFile, JSON.stringify(feed));
  // #14167: the spawned server must not touch the real machine — its
  // background jobs (the pm-core quota/fleet-feed schedulers, the Meta
  // renewal keeper and the snapshot/hot-list consumers) run real
  // collection unless their seams are disconnected. The fixture
  // subcommand answers with empty provider readings so the gatherer's
  // files stay in the tmp dir; the renewal keeper is disabled; the Turso
  // sink is off; the audit logs and snapshots land beside the feed.
  const subcommandPath = join(dir, "quota-subcommand.mjs");
  const providerResult = {
    providerId: "openai",
    providerName: "OpenAI",
    ok: true,
    configured: true,
    fetchedAt: Date.parse(now),
    failureClass: "no-data",
    lastSuccessAt: Date.parse(now),
    usage: { windows: { weekly: { usedPercent: null } } },
  };
  // Fixture subcommand: answers with one valid provider row so the
  // gatherer exercises its write paths (heartbeat, current, anomaly
  // state) without touching real provider credentials or the network.
  await writeFile(subcommandPath, `console.log(${JSON.stringify(JSON.stringify([providerResult]))});\n`, "utf8");
  const port = 5100 + Math.floor(Math.random() * 800);
  packageDirBefore = (await readdir(packageDir)).sort();
  proc = spawn({
    cmd: ["bun", "run", join(packageDir, "src", "server.ts")],
    cwd: dir,
    env: {
      ...Bun.env,
      HOME: dir,
      PM_AGG_PORT: String(port),
      PM_AGG_FRESHNESS_PORT: String(port + 1000),
      PM_AGG_SOURCE: "live",
      PM_AGG_FEED_FILE: feedFile,
      PM_AGG_QUOTA_CURRENT_FILE: join(dir, "quota-current.json"),
      QUOTA_CURRENT_PATH: join(dir, "quota-current.json"),
      QUOTA_HEARTBEAT_PATH: join(dir, "quota-gatherer-heartbeat.json"),
      QUOTA_GATHERER_STATE_PATH: join(dir, "quota-gatherer-state.json"),
      QUOTA_SNAPSHOTS_DIR: join(dir, "snapshots"),
      QUOTA_FORECAST_PATH: join(dir, "quota-forecast.json"),
      QUOTA_SUBCOMMAND: `${process.execPath} ${subcommandPath}`,
      QUOTA_SUBCOMMAND_TIMEOUT_MS: "5000",
      META_RENEWAL_DISABLED: "1",
      OPENDESIGN_RENEWAL_DISABLED: "1",
      TURSO_QUOTA_SINK: "0",
      META_RENEWAL_STATE_PATH: join(dir, "meta-renewal.json"),
      PM_CORE_JOBS_AUDIT_PATH: join(dir, "pm-core-jobs.jsonl"),
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
}, 60_000);

afterAll(() => {
  proc?.kill();
});

describe("/api/health provenance priming (#1267)", () => {
  test("the first-ever response reports the live facets truthfully", () => {
    // #14062: the status follows the degraded flag. This live-source
    // server's probes reach nothing, so it is degraded and says so.
    expect(firstHealth?.degraded).toBe(true);
    expect(firstHealth?.status).toBe("degraded");
    expect(firstHealth?.source.quota).toContain("live (fleet feed");
    expect(firstHealth?.source.quota).toContain("1 credential row(s)");
    expect(firstHealth?.source.quota).toContain("1 active session(s)");
    expect(firstHealth?.source.orchestration).toContain("live (orchestration section orch-health-test");
    expect(firstHealth?.source.registry).toContain("1 session records");
    expect(firstHealth?.source.registry).not.toBe("fixtures");
    expect(firstHealth?.sections?.orchestration.state).toBe("live");
    expect(firstHealth?.sections?.orchestration.runId).toBe("orch-health-test");
  });

  test("the run leaves no files in the package directory", async () => {
    await Bun.sleep(500);
    expect((await readdir(packageDir)).sort()).toEqual(packageDirBefore);
  });
});
