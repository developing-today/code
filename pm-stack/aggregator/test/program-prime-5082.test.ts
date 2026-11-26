import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawn, type Subprocess } from "bun";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { buildQuotaAnalytics, primeAttemptAggregates } from "../src/analytics-store.ts";
import type { QuotaPool, QuotaView, UsageView } from "../src/model.ts";
import { credentialKey } from "../src/route-rollup.ts";

// #5082 follow-up (2026-11-13): the /program cold-path stall. The
// program assembly was the one major assembly the boot warm-up never
// warmed, and the warm-up itself monopolized the single event loop:
// its synchronous first buildQuotaAnalytics measured ~12 s, and the
// first /api/program.json after a restart measured 26.4 s against
// ~0.03 s warm — while the program legs themselves total ~0.3 s and
// warm TTL-expiry passes serve stale in ~0.02 s. The fix primes the
// aggregates cache through the day-sliced refresh (never the
// synchronous cold build at boot) and warms programSources with the
// other assemblies.
//
// The unit section pins the prime contract against a stand-in for the
// OpenCode server's rollup read (#2795); the route section pins that a booted server's
// first /api/program.json is served by the warmed path.

const emptyQuota = (): QuotaView =>
  ({ pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } }) as QuotaView;
const emptyUsage = (): UsageView =>
  ({
    services: [],
    uninstrumentedServices: [],
    payg: [],
    charges: [],
    attention: [],
    domains: null,
    subscriptions: [],
    history: {},
  }) as unknown as UsageView;

interface FixtureAttempt {
  time_started: number;
  provider_id: string;
  credential_id: string | null;
  model_id: string;
  variant: string | null;
  outcome: string;
  tokens_input: number;
  tokens_output: number;
  tokens_cache_read: number;
  tokens_cache_write: number;
  cost_usd: number | null;
}

/** A stand-in for the OpenCode server's rollup read (#2795) over fixture
 * rows: same query parameters, same row shape, local-clock buckets,
 * credentials served only as credentialKey. */
function fakeRollupServer(rows: FixtureAttempt[], seen: string[]) {
  return Bun.serve({
    port: 0,
    fetch(req) {
      const url = new URL(req.url);
      seen.push(url.pathname + url.search);
      if (url.pathname !== "/api/route/attempts/rollup") return new Response("not found", { status: 404 });
      if (req.headers.get("authorization") !== `Basic ${Buffer.from("opencode:pw").toString("base64")}`)
        return new Response("unauthorized", { status: 401 });
      const since = Number(url.searchParams.get("since") ?? 0);
      const until = url.searchParams.has("until") ? Number(url.searchParams.get("until")) : Infinity;
      const bucket = url.searchParams.get("bucket") ?? "none";
      const group = (url.searchParams.get("group") ?? "").split(",").filter(Boolean);
      const bucketOf = (ms: number): string | number | null => {
        const d = new Date(ms);
        if (bucket === "day")
          return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
        if (bucket === "hourOfDay") return d.getHours();
        if (bucket === "dayOfWeek") return d.getDay();
        return null;
      };
      const out = new Map<string, Record<string, unknown> & { attempts: number }>();
      for (const r of rows) {
        if (r.time_started < since || r.time_started >= until) continue;
        const dims: Record<string, unknown> = {};
        if (group.includes("provider")) dims.providerID = r.provider_id;
        if (group.includes("model")) dims.modelID = r.model_id;
        if (group.includes("variant")) dims.variant = r.variant;
        if (group.includes("credential"))
          dims.credentialKey = r.credential_id === null ? null : credentialKey(r.credential_id);
        const b = bucketOf(r.time_started);
        const key = JSON.stringify([b, dims]);
        const acc =
          out.get(key) ??
          ({
            bucket: b,
            ...dims,
            attempts: 0,
            successes: 0,
            tokensInput: 0,
            tokensOutput: 0,
            tokensReasoning: 0,
            tokensCacheRead: 0,
            tokensCacheWrite: 0,
            tokensCombined: 0,
            costUsd: 0,
            costRows: 0,
            credentialRows: 0,
            first: r.time_started,
            last: r.time_started,
          } as Record<string, unknown> & { attempts: number });
        const add = (k: string, v: number) => (acc[k] = (acc[k] as number) + v);
        acc.attempts += 1;
        add("successes", r.outcome === "success" ? 1 : 0);
        add("tokensInput", r.tokens_input);
        add("tokensOutput", r.tokens_output);
        add("tokensCacheRead", r.tokens_cache_read);
        add("tokensCacheWrite", r.tokens_cache_write);
        add("tokensCombined", r.tokens_input + r.tokens_output + r.tokens_cache_read + r.tokens_cache_write);
        add("costUsd", r.cost_usd ?? 0);
        add("costRows", r.cost_usd === null ? 0 : 1);
        add("credentialRows", r.credential_id === null ? 0 : 1);
        acc.first = Math.min(acc.first as number, r.time_started);
        acc.last = Math.max(acc.last as number, r.time_started);
        out.set(key, acc);
      }
      return Response.json({ data: [...out.values()] });
    },
  });
}

describe("attempt-aggregates boot prime (#5082, read through the route API since #2795)", () => {
  const dir = mkdtempSync(join(tmpdir(), "agg-prime-"));
  const now = Date.now();
  const rowOld = now - 2 * 86_400_000;
  const rowNew = now - 3_600_000;
  const row = (time_started: number): FixtureAttempt => ({
    time_started,
    provider_id: "prov-a",
    credential_id: "cred-1",
    model_id: "model-x",
    variant: null,
    outcome: "success",
    tokens_input: 100,
    tokens_output: 50,
    tokens_cache_read: 10,
    tokens_cache_write: 5,
    cost_usd: 0.25,
  });
  const seen: string[] = [];
  const server = fakeRollupServer([row(rowOld), row(rowNew), row(rowNew)], seen);
  const target = { baseUrl: `http://127.0.0.1:${server.port}`, password: "pw" };
  const quotaCurrentPath = join(dir, "quota-current.json");

  beforeAll(() => {
    writeFileSync(quotaCurrentPath, JSON.stringify({ providers: [{ providerId: "mistral", credentialId: "cred-1" }] }));
  });

  afterAll(() => {
    server.stop(true);
    rmSync(dir, { recursive: true, force: true });
  });

  test("before the prime lands the section is unavailable, not a blocking cold build", () => {
    const analytics = buildQuotaAnalytics({ quota: emptyQuota(), usage: emptyUsage(), quotaCurrentPath, now });
    expect(analytics.dbAvailable).toBe(false);
  });

  test("prime fills the cache from the route API; the build then serves the primed coverage and pool legs", async () => {
    let settle!: () => void;
    const settled = new Promise<void>((resolve) => {
      settle = resolve;
    });
    expect(primeAttemptAggregates(target, settle)).toBe(true);
    // A second prime while the first is in flight starts nothing.
    expect(primeAttemptAggregates(target)).toBe(false);
    await settled;

    const quota = emptyQuota();
    quota.pools = [
      { id: "mistral", provider: "mistral", plan: "test", state: "ok", windows: [] } as unknown as QuotaPool,
    ];
    const analytics = buildQuotaAnalytics({ quota, usage: emptyUsage(), quotaCurrentPath, routeApi: target, now });
    expect(analytics.dbAvailable).toBe(true);
    expect(analytics.attemptsFrom).toBe(new Date(rowOld).toISOString());
    expect(analytics.attemptsTo).toBe(new Date(rowNew).toISOString());
    expect(analytics.totals.last7d.calls).toBe(3);
    expect(analytics.totals.last7d.costRows).toBe(3);
    expect(analytics.credentialCoveragePct).toBe(100);
    expect(analytics.hourly7d.reduce((n, h) => n + h.calls, 0)).toBe(3);
    expect(analytics.hourly7d.reduce((n, h) => n + h.tokens, 0)).toBe(3 * 165);
    expect(analytics.dowHistory.reduce((n, d) => n + d.calls, 0)).toBe(3);
    // The credential join still attributes the pool, through the key alone.
    expect(analytics.pools[0]?.burn7d?.calls).toBe(3);
    expect(analytics.pools[0]?.legBasis).toBe("credential");
    // Every read went to the rollup endpoint and none named a credential id.
    expect(seen.length).toBeGreaterThan(0);
    expect(seen.every((p) => p.startsWith("/api/route/attempts/rollup?"))).toBe(true);
    expect(seen.some((p) => p.includes("cred-1"))).toBe(false);

    // Cache populated: a later prime is a no-op whose callback fires.
    let fired = false;
    expect(
      primeAttemptAggregates(target, () => {
        fired = true;
      }),
    ).toBe(false);
    expect(fired).toBe(true);
  });

  test("prime with no server to ask is a fail-soft no-op", async () => {
    let fired = false;
    expect(
      primeAttemptAggregates(null, () => {
        fired = true;
      }),
    ).toBe(false);
    expect(fired).toBe(true);
  });
});

describe("booted server serves the program path (#5082)", () => {
  let proc: Subprocess<"ignore", "pipe", "inherit"> | null = null;
  let port = 0;

  beforeAll(async () => {
    port = 5100 + Math.floor(Math.random() * 1000);
    proc = spawn({
      cmd: ["bun", "run", "src/server.ts"],
      cwd: import.meta.dir + "/..",
      env: {
        ...Bun.env,
        PM_AGG_PORT: String(port),
        PM_AGG_FRESHNESS_PORT: String(port + 2000),
        PM_AGG_SOURCE: "fixtures",
      },
      stdout: "pipe",
      stderr: "inherit",
    });
    const deadline = Date.now() + 10_000;
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

  test("first /api/program.json after boot answers 200 with the program payload", async () => {
    const res = await fetch(`http://127.0.0.1:${port}/api/program.json`);
    expect(res.status).toBe(200);
    const body = (await res.json()) as { projects?: unknown[] };
    expect(Array.isArray(body.projects)).toBe(true);
  });
});
