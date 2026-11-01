import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawn, type Subprocess } from "bun";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import type { FleetFeed, SectionDiagnosis } from "../src/feed.ts";
import { buildHealth, createProbeCache, recordRender, resolveBuildVersion, type HealthInputs, type ProbeFetch } from "../src/health.ts";
import { pendingCommentCounts } from "../src/comments.ts";
import type { DashComment } from "../src/model.ts";
import { renderHealth } from "../src/render.ts";

// #1147/#1137: health assembly — service probes (injectable fetcher,
// 60 s cache), feed section ages, job runs, the render registry, and
// the fixtures-mode story. The degraded states are proven here with
// fake fetchers; no service is ever killed on the laptop for a test.

const NOW = Date.parse("2026-10-27T06:00:00Z");
const cfg = loadConfig({});

function feedAt(ageMs: number): FleetFeed {
  const generatedAt = new Date(NOW - ageMs).toISOString();
  return {
    version: 1,
    generatedAt,
    quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
    attention: [],
    orchestration: {
      version: 1,
      generatedAt,
      runId: "orch-health-unit",
      provenance: { sources: ["unit test"] },
      tiers: { needsDrew: [], needsOrchestrator: [] },
      dependencies: [],
      goals: [],
      actions: [],
    },
    dns: { generatedAt, source: "unit", totalAnnualListUsd: 0, entries: [] },
    registry: [],
    projects: [],
  } as unknown as FleetFeed;
}

const LIVE_DIAGNOSIS: SectionDiagnosis = {
  state: "live",
  runId: "orch-health-unit",
  generatedAt: new Date(NOW - 60_000).toISOString(),
  ageMs: 60_000,
};

function inputs(over: Partial<HealthInputs> = {}): HealthInputs {
  return {
    mode: "live",
    cfg,
    feed: feedAt(4 * 60_000),
    feedFileMtimeMs: NOW - 4 * 60_000,
    diagnosis: LIVE_DIAGNOSIS,
    ...over,
  };
}

/** A fetcher that answers per-URL and records every call. */
function fakeFetcher(handler: (url: string) => { status: number } | Error) {
  const calls: string[] = [];
  const fetcher: ProbeFetch = async (url) => {
    calls.push(url);
    const out = handler(url);
    if (out instanceof Error) throw out;
    return out;
  };
  return { fetcher, calls };
}

const allUp = () => ({ status: 200 });

function service(view: Awaited<ReturnType<typeof buildHealth>>, id: string) {
  const s = view.services.find((x) => x.def.id === id);
  if (!s) throw new Error(`no service ${id}`);
  return s;
}

describe("buildHealth probes", () => {
  test("a stopped collector is visible on the health page by the next five-minute interval", async () => {
    // An isolated stand-in collector writes only a temporary feed. Freeze the
    // observation clock immediately after its last write and advance it to the
    // next interval: this tests the worst-case stop without a five-minute wait.
    const stoppedAt = NOW;
    const dir = await mkdtemp(join(tmpdir(), "collector-stop-"));
    const path = join(dir, "feed.json");
    const collector = spawn({ cmd: [process.execPath, "-e", "await Bun.write(process.argv[1], process.argv[2]); setInterval(() => {}, 1000)", path, JSON.stringify(feedAt(0))], stdout: "ignore", stderr: "inherit" });
    try {
      for (let i = 0; i < 100 && !(await Bun.file(path).exists()); i++) await Bun.sleep(10);
      const feed = await Bun.file(path).json() as FleetFeed;
      collector.kill();
      await collector.exited;
      const { fetcher } = fakeFetcher(allUp);
      const opts = { fetcher, cache: createProbeCache() };
      const before = await buildHealth(inputs({ feed }), { ...opts, now: stoppedAt });
      expect(service(before, "fleet-feed").state).toBe("up");
      const view = await buildHealth(inputs({ feed }), { ...opts, now: stoppedAt + 5 * 60_000 });
      expect(service(view, "fleet-feed").state).toBe("stale");
      expect(view.degraded).toBe(true);
      const html = renderHealth(view, {} as Parameters<typeof renderHealth>[1]);
      expect(html).toContain("past the 5m freshness threshold");
      expect(html).toContain("stale");
    } finally {
      collector.kill();
      await collector.exited;
      await rm(dir, { recursive: true, force: true });
    }
  });
  test("all services up: 200s and Grist's 302 prove life; projector never probed", async () => {
    const { fetcher, calls } = fakeFetcher((url) =>
      url.startsWith("http://127.0.0.1:8484") ? { status: 302 } : allUp(),
    );
    const view = await buildHealth(inputs(), { fetcher, cache: createProbeCache(), now: NOW });
    expect(service(view, "kaneo").state).toBe("up");
    expect(service(view, "grist").state).toBe("up");
    expect(service(view, "grist").statusCode).toBe(302);
    expect(service(view, "ready-for-agent").state).toBe("up");
    expect(service(view, "pad").state).toBe("up");
    expect(service(view, "aggregator").state).toBe("self");
    expect(service(view, "kaneo-postgres").state).toBe("up");
    expect(service(view, "fleet-feed").state).toBe("up");
    expect(service(view, "orchestration").state).toBe("up");
    const projector = service(view, "projector");
    expect(projector.state).toBe("off");
    expect(projector.ok).toBeNull();
    expect(calls.some((u) => u.includes("7801"))).toBe(false);
    expect(view.degraded).toBe(false);
  });

  test("a throwing probe renders down with its error; the inferred service says unknown, not down", async () => {
    const { fetcher } = fakeFetcher((url) =>
      url.startsWith("http://127.0.0.1:5173") ? new Error("connect ECONNREFUSED") : allUp(),
    );
    const view = await buildHealth(inputs(), { fetcher, cache: createProbeCache(), now: NOW });
    const kaneo = service(view, "kaneo");
    expect(kaneo.state).toBe("down");
    expect(kaneo.ok).toBe(false);
    expect(kaneo.detail).toContain("ECONNREFUSED");
    const pg = service(view, "kaneo-postgres");
    expect(pg.state).toBe("unknown");
    expect(pg.ok).toBeNull();
    expect(view.degraded).toBe(true);
  });

  test("a timeout renders down as a timeout; an unexpected status renders down with the status", async () => {
    const timeout = new Error("timed out");
    timeout.name = "TimeoutError";
    const { fetcher } = fakeFetcher((url) => {
      if (url.startsWith("http://127.0.0.1:6056")) return timeout;
      if (url.startsWith("http://127.0.0.1:7777")) return { status: 503 };
      return allUp();
    });
    const view = await buildHealth(inputs(), { fetcher, cache: createProbeCache(), now: NOW });
    expect(service(view, "ready-for-agent").detail).toContain("timed out");
    const pad = service(view, "pad");
    expect(pad.state).toBe("down");
    expect(pad.detail).toContain("HTTP 503");
  });

  test("the 60 s probe cache suppresses a second probe, then expires", async () => {
    const { fetcher, calls } = fakeFetcher(allUp);
    const cache = createProbeCache();
    await buildHealth(inputs(), { fetcher, cache, now: NOW });
    const firstCount = calls.length;
    expect(firstCount).toBeGreaterThan(0);
    await buildHealth(inputs(), { fetcher, cache, now: NOW + 30_000 });
    expect(calls.length).toBe(firstCount);
    await buildHealth(inputs(), { fetcher, cache, now: NOW + 61_000 });
    expect(calls.length).toBe(firstCount * 2);
  });

  test("absent orchestration section is a named state, not a silent zero", async () => {
    const { fetcher } = fakeFetcher(allUp);
    const view = await buildHealth(inputs({ diagnosis: { state: "absent" } }), {
      fetcher,
      cache: createProbeCache(),
      now: NOW,
    });
    const orch = service(view, "orchestration");
    expect(orch.state).toBe("absent");
    expect(orch.ok).toBe(false);
    expect(view.jobs.orchestration.state).toBe("absent");
    expect(view.degraded).toBe(true);
  });

  test("invalid section names its failed checks; stale feed marks the collector stale", async () => {
    const { fetcher } = fakeFetcher(allUp);
    const view = await buildHealth(
      inputs({
        feed: feedAt(20 * 60_000),
        diagnosis: { state: "invalid", failedChecks: ["provenance"] },
      }),
      { fetcher, cache: createProbeCache(), now: NOW },
    );
    const orch = service(view, "orchestration");
    expect(orch.state).toBe("invalid");
    expect(orch.detail).toContain("provenance");
    expect(service(view, "fleet-feed").state).toBe("stale");
    expect(view.degraded).toBe(true);
    const orchSection = view.feed.sections.find((s) => s.key === "orchestration");
    expect(orchSection?.present).toBe(true);
    expect(orchSection?.ageMs).toBe(20 * 60_000);
    const saas = view.feed.sections.find((s) => s.key === "saas");
    expect(saas?.present).toBe(false);
  });

  test("a feed stamped far in the future is stale, never up/fresh (#1564)", async () => {
    const { fetcher } = fakeFetcher(allUp);
    const view = await buildHealth(inputs({ feed: feedAt(-30 * 86_400_000) }), {
      fetcher,
      cache: createProbeCache(),
      now: NOW,
    });
    const ff = service(view, "fleet-feed");
    expect(ff.state).toBe("stale");
    expect(ff.ok).toBe(false);
    expect(ff.detail).toContain("in the future");
    expect(view.degraded).toBe(true);
  });

  test("fixtures mode never probes and reports fixtures for every service", async () => {
    const { fetcher, calls } = fakeFetcher(allUp);
    const view = await buildHealth(
      inputs({ mode: "fixtures", feed: null, feedFileMtimeMs: null, diagnosis: undefined }),
      {
        fetcher,
        cache: createProbeCache(),
        now: NOW,
      },
    );
    expect(calls.length).toBe(0);
    expect(view.services.every((s) => s.state === "fixtures")).toBe(true);
    expect(view.degraded).toBe(false);
  });

  test("pages come from the render registry — rendered vs never rendered", async () => {
    recordRender("/today", true, NOW - 120_000);
    const { fetcher } = fakeFetcher(allUp);
    const view = await buildHealth(inputs(), { fetcher, cache: createProbeCache(), now: NOW });
    const today = view.pages.find((p) => p.route === "/today");
    expect(today?.lastRenderedAt).toBe(new Date(NOW - 120_000).toISOString());
    expect(today?.ok).toBe(true);
    const health = view.pages.find((p) => p.route === "/health");
    expect(health?.title).toBe("Health");
    const unrendered = view.pages.find((p) => p.route === "/pages");
    // /pages may or may not have been recorded by another test in this
    // process; assert only the shape contract for unrendered entries.
    if (unrendered && unrendered.lastRenderedAt === null) {
      expect(unrendered.ok).toBeNull();
      expect(unrendered.ageMs).toBeNull();
    }
  });
});

// --- Boot: the fixture server serves /health and the extended twin ---------

let proc: Subprocess | null = null;
let port = 0;

beforeAll(async () => {
  port = 6100 + Math.floor(Math.random() * 800);
  proc = spawn({
    cmd: ["bun", "run", "src/server.ts"],
    cwd: import.meta.dir + "/..",
    env: { ...Bun.env, PM_AGG_PORT: String(port), PM_AGG_SOURCE: "fixtures" },
    stdout: "pipe",
    stderr: "inherit",
  });
  const deadline = Date.now() + 8000;
  let ready = false;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/api/health`);
      if (res.ok) {
        ready = true;
        break;
      }
    } catch {
      /* not up yet */
    }
    await Bun.sleep(120);
  }
  expect(ready).toBe(true);
});

afterAll(() => {
  proc?.kill();
});

describe("health page + extended /api/health (fixtures boot)", () => {
  test("/health renders the topology table with honest fixture states", async () => {
    const res = await fetch(`http://127.0.0.1:${port}/health`);
    expect(res.status).toBe(200);
    const html = await res.text();
    expect(html).toContain("Services &amp; topology");
    expect(html).toContain("ready-for-agent");
    expect(html).toContain("Kaneo Postgres");
    expect(html).toContain("off by config");
    expect(html).toContain("fixtures");
    expect(html).toContain("not rendered since this process started");
  });

  test("/api/health keeps its old keys and gains services/feed/pages/jobs", async () => {
    const res = await fetch(`http://127.0.0.1:${port}/api/health`);
    expect(res.status).toBe(200);
    const body = (await res.json()) as {
      status: string;
      source: { board: string };
      services: { id: string; state: string }[];
      feed: { generatedAt: unknown };
      pages: { route: string; lastRenderedAt: unknown }[];
      jobs: { orchestration: { state: string } };
    };
    expect(body.status).toBe("ok");
    expect(body.source.board).toBe("fixtures");
    expect(Array.isArray(body.services)).toBe(true);
    expect(body.services.find((s) => s.id === "projector")?.state).toBe("fixtures");
    expect("generatedAt" in body.feed).toBe(true);
    expect(body.pages.some((p) => p.route === "/health")).toBe(true);
    expect(body.jobs.orchestration.state).toBeDefined();
  });

  test("/pages lists the health page from the manifest", async () => {
    const res = await fetch(`http://127.0.0.1:${port}/pages`);
    expect(res.status).toBe(200);
    const html = await res.text();
    expect(html).toContain('href="/health"');
    expect(html).toContain("/api/health");
  });
});

// --- #1145: dashboard comment pending counts on the health view ----------

function dashComment(over: Partial<DashComment>): DashComment {
  return {
    id: "dc-test",
    objectRef: "gh:developing-today/code#1145",
    kind: "note",
    author: { kind: "drew", id: "drew", label: "Drew" },
    body: "test",
    createdAt: "2026-10-29T00:00:00Z",
    ...over,
  } as DashComment;
}

describe("pending comment counts (#1145)", () => {
  test("pendingCommentCounts counts only unworked message/github comments", () => {
    const counts = pendingCommentCounts([
      dashComment({ kind: "message", triageState: "new" }),
      dashComment({ kind: "message", triageState: "new" }),
      dashComment({ kind: "message", triageState: "triaged" }),
      dashComment({
        kind: "github",
        githubState: "pending",
        githubRef: "developing-today/code#1145" as DashComment["githubRef"],
      }),
      dashComment({
        kind: "github",
        githubState: "posted",
        githubRef: "developing-today/code#1145" as DashComment["githubRef"],
      }),
      dashComment({
        kind: "github",
        githubState: "failed",
        githubRef: "developing-today/code#1145" as DashComment["githubRef"],
      }),
      dashComment({ kind: "note" }),
    ]);
    expect(counts).toEqual({ pendingMessages: 2, pendingGithub: 1, total: 7 });
  });

  test("buildHealth carries the counts into the view and defaults to zero", async () => {
    const { fetcher } = fakeFetcher(allUp);
    const withCounts = await buildHealth(inputs({ comments: { pendingMessages: 3, pendingGithub: 2, total: 9 } }), {
      fetcher,
      cache: createProbeCache(),
      now: NOW,
    });
    expect(withCounts.comments).toEqual({ pendingMessages: 3, pendingGithub: 2, total: 9 });
    const without = await buildHealth(inputs(), { fetcher, cache: createProbeCache(), now: NOW });
    expect(without.comments).toEqual({ pendingMessages: 0, pendingGithub: 0, total: 0 });
  });

  test("renderHealth renders the pending counts", async () => {
    const { fetcher } = fakeFetcher(allUp);
    const view = await buildHealth(inputs({ comments: { pendingMessages: 3, pendingGithub: 2, total: 9 } }), {
      fetcher,
      cache: createProbeCache(),
      now: NOW,
    });
    const html = renderHealth(view, {
      board: "fixtures",
      roadmap: "fixtures",
      items: "fixtures",
      comments: "fixtures",
      quota: "fixtures",
      usage: "fixtures",
      attention: "fixtures",
      orchestration: "fixtures",
      dns: "fixtures",
      registry: "fixtures",
      aa: "fixtures",
    } as Parameters<typeof renderHealth>[1]);
    expect(html).toContain("3 messages awaiting triage");
    expect(html).toContain("2 GitHub comments awaiting posting");
    expect(html).toContain("9 comments in the store");
    expect(html).toContain("a pending count that does not drain means work arrives faster than it is cleared, or the triage/poster loop has stopped; check the loop's last run before acting");
    expect(html).not.toContain("a pending count that never drains means the triage/poster loop is not running");
  });
});

describe("buildHealth version marker (#1150)", () => {
  test("exposes the deployed build version and defaults to unknown", async () => {
    const { fetcher } = fakeFetcher(allUp);
    const deployed = await buildHealth(inputs({ version: "abc1234deadbeef" }), {
      fetcher,
      cache: createProbeCache(),
      now: NOW,
    });
    expect(deployed.version).toBe("abc1234deadbeef");
    const unmarked = await buildHealth(inputs(), { fetcher, cache: createProbeCache(), now: NOW });
    expect(unmarked.version).toBe(resolveBuildVersion());
    expect(resolveBuildVersion({})).toBe("unknown");
    expect(resolveBuildVersion({ PM_AGG_BUILD_VERSION: "  v1  " })).toBe("v1");
    expect(resolveBuildVersion({ PM_AGG_BUILD_VERSION: "   " })).toBe("unknown");
  });
});
