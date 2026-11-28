import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import { spawn, type Subprocess } from "bun";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { buildQuotaAnalytics, primeAttemptAggregates } from "../src/analytics-store.ts";
import type { QuotaView, UsageView } from "../src/model.ts";

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
// The unit section pins the prime contract against a fixture
// route_attempt DB; the route section pins that a booted server's
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

describe("attempt-aggregates boot prime (#5082)", () => {
  const dir = mkdtempSync(join(tmpdir(), "agg-prime-"));
  const dbPath = join(dir, "fixture.db");
  const now = Date.now();
  const rowOld = now - 2 * 86_400_000;
  const rowNew = now - 3_600_000;

  beforeAll(() => {
    const db = new Database(dbPath, { create: true });
    db.run(`CREATE TABLE route_attempt (
      time_started INTEGER, provider_id TEXT, credential_id TEXT,
      model_id TEXT, variant TEXT, outcome TEXT,
      tokens_input INTEGER, tokens_output INTEGER, tokens_reasoning INTEGER,
      tokens_cache_read INTEGER, tokens_cache_write INTEGER, cost_usd REAL
    )`);
    const ins = db.query(
      `INSERT INTO route_attempt VALUES (?, 'prov-a', 'cred-1', 'model-x', NULL, 'success', 100, 50, 0, 10, 5, 0.25)`,
    );
    ins.run(rowOld);
    ins.run(rowNew);
    ins.run(rowNew);
    db.close();
  });

  afterAll(() => {
    rmSync(dir, { recursive: true, force: true });
  });

  test("prime fills the cache through the sliced refresh; the build then serves the primed coverage", async () => {
    let settle!: () => void;
    const settled = new Promise<void>((resolve) => {
      settle = resolve;
    });
    expect(primeAttemptAggregates(dbPath, settle)).toBe(true);
    // A second prime while the first is in flight starts nothing.
    expect(primeAttemptAggregates(dbPath)).toBe(false);
    await settled;

    const analytics = buildQuotaAnalytics({
      quota: emptyQuota(),
      usage: emptyUsage(),
      quotaCurrentPath: join(dir, "no-such-quota-current.json"),
      opencodeDbPath: dbPath,
      now,
    });
    expect(analytics.dbAvailable).toBe(true);
    expect(analytics.attemptsFrom).toBe(new Date(rowOld).toISOString());
    expect(analytics.attemptsTo).toBe(new Date(rowNew).toISOString());
    expect(analytics.totals.last7d.calls).toBe(3);

    // Cache populated: a later prime is a no-op whose callback fires.
    let fired = false;
    expect(
      primeAttemptAggregates(dbPath, () => {
        fired = true;
      }),
    ).toBe(false);
    expect(fired).toBe(true);
  });

  test("prime against a missing DB is a fail-soft no-op", async () => {
    let fired = false;
    expect(
      primeAttemptAggregates(join(dir, "absent.db"), () => {
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
