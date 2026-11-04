import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawn, type Subprocess } from "bun";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { TODAY_CACHE_TTL_MS, TodayCache } from "../src/today-cache.ts";

// #1179: repeated warm /today + /frag/today requests must not repeat
// the live-source fan-out, and concurrent cold requests must share one
// in-flight load — while failures, provenance, and degraded state stay
// truthful. Unit coverage pins the cache contract; the route section
// below pins the wiring through a booted fixture-mode server.

describe("TodayCache cold miss / warm hit / expiry", () => {
  test("cold miss runs the loader and caches the result", async () => {
    let now = 1_000_000;
    let calls = 0;
    const cache = new TodayCache<string>(10_000, () => now);
    const first = await cache.get(async () => {
      calls++;
      return "v1";
    });
    expect(first).toBe("v1");
    expect(calls).toBe(1);
    // Warm hit: same value, loader not re-run.
    const second = await cache.get(async () => {
      calls++;
      return "v2";
    });
    expect(second).toBe("v1");
    expect(calls).toBe(1);
  });

  test("expiry triggers a fresh read", async () => {
    let now = 1_000_000;
    let calls = 0;
    const cache = new TodayCache<string>(10_000, () => now);
    await cache.get(async () => {
      calls++;
      return `v${calls}`;
    });
    now += TODAY_CACHE_TTL_MS - 1;
    expect(await cache.get(async () => "stale-never")).toBe("v1");
    expect(calls).toBe(1);
    // Exactly at the TTL the entry is stale — a fresh read runs.
    now += 1;
    expect(await cache.get(async () => `v${++calls}`)).toBe("v2");
    expect(calls).toBe(2);
  });

  test("clear() forces the next read to miss", async () => {
    let calls = 0;
    const cache = new TodayCache<string>();
    await cache.get(async () => `v${++calls}`);
    cache.clear();
    expect(await cache.get(async () => `v${++calls}`)).toBe("v2");
    expect(calls).toBe(2);
  });
});

describe("TodayCache concurrent coalescing", () => {
  test("concurrent cold requests share one in-flight load", async () => {
    let calls = 0;
    let release!: (v: string) => void;
    const gate = new Promise<string>((resolve) => {
      release = resolve;
    });
    const cache = new TodayCache<string>();
    const loader = async () => {
      calls++;
      return gate;
    };
    const pending = [cache.get(loader), cache.get(loader), cache.get(loader)];
    // Let all three attach to the in-flight load before it resolves.
    await Bun.sleep(10);
    expect(calls).toBe(1);
    release("shared");
    const results = await Promise.all(pending);
    expect(results).toEqual(["shared", "shared", "shared"]);
    expect(calls).toBe(1);
    // The coalesced result is now the warm value.
    expect(await cache.get(async () => "never")).toBe("shared");
    expect(calls).toBe(1);
  });
});

describe("TodayCache failure and degraded-state behavior", () => {
  test("a throwing loader never poisons the cache; the next request retries", async () => {
    let calls = 0;
    const cache = new TodayCache<string>();
    await expect(
      cache.get(async () => {
        calls++;
        throw new Error("kaneo unreachable");
      }),
    ).rejects.toThrow("kaneo unreachable");
    expect(calls).toBe(1);
    // Retry runs the loader again instead of serving the failure…
    expect(await cache.get(async () => `recovered-${++calls}`)).toBe("recovered-2");
    expect(calls).toBe(2);
    // …and the successful read is cached normally.
    expect(await cache.get(async () => "never")).toBe("recovered-2");
    expect(calls).toBe(2);
  });

  test("concurrent waiters on a failed load all reject and release the slot", async () => {
    let calls = 0;
    let reject!: (e: Error) => void;
    const gate = new Promise<string>((_, rej) => {
      reject = rej;
    });
    const cache = new TodayCache<string>();
    const loader = async () => {
      calls++;
      return gate;
    };
    const pending = [cache.get(loader), cache.get(loader)];
    await Bun.sleep(10);
    expect(calls).toBe(1);
    reject(new Error("grist timeout"));
    await expect(Promise.all(pending)).rejects.toThrow("grist timeout");
    expect(
      await cache.get(async () => {
        calls++;
        return "fresh";
      }),
    ).toBe("fresh");
    expect(calls).toBe(2);
  });

  test("fail-soft degraded values are served truthfully within the TTL, then refreshed", async () => {
    // server.ts's loaders catch per-facet: a quota outage resolves to
    // { quotaUnavailable: true }, which the cache must preserve as-is
    // (not mask, not throw) until expiry triggers a fresh read.
    let now = 1_000_000;
    let outage = true;
    const cache = new TodayCache<{ quotaUnavailable: boolean }>(10_000, () => now);
    const loader = async () => ({ quotaUnavailable: outage });
    expect((await cache.get(loader)).quotaUnavailable).toBe(true);
    outage = false; // the source recovers behind the cache
    expect((await cache.get(loader)).quotaUnavailable).toBe(true);
    now += TODAY_CACHE_TTL_MS;
    expect((await cache.get(loader)).quotaUnavailable).toBe(false);
  });

  test("warm hits serve the last read's stamps, not fresh ones (bounded lag)", async () => {
    // Provenance STATE stays live per request (server.ts reads
    // prov()/provDetail() outside the cache); the cached payload keeps
    // its read-time stamps, lagging by at most the TTL.
    let now = 1_000_000;
    const cache = new TodayCache<{ readAt: number }>(10_000, () => now);
    const first = await cache.get(async () => ({ readAt: now }));
    now += 5_000;
    const second = await cache.get(async () => ({ readAt: now }));
    expect(second).toBe(first);
    expect(second.readAt).toBe(1_000_000);
  });
});

let proc: Subprocess | undefined;
let base = "";

beforeAll(async () => {
  const dir = await mkdtemp(join(tmpdir(), "pm-today-cache-1179-"));
  const port = 6000 + Math.floor(Math.random() * 800);
  base = `http://127.0.0.1:${port}`;
  proc = spawn({
    cmd: ["bun", "run", "src/server.ts"],
    cwd: join(import.meta.dir, ".."),
    env: {
      ...Bun.env,
      PM_AGG_PORT: String(port),
      PM_AGG_FEED_FILE: join(dir, "no-feed.json"),
      PM_AGG_REGISTRY_FILE: join(dir, "registry.json"),
      PM_AGG_COMMENTS_FILE: join(dir, "comments.json"),
      PM_AGG_ACTIONS_FILE: join(dir, "actions.json"),
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
  const deadline = Date.now() + 20_000;
  let up = false;
  while (Date.now() < deadline && !up) {
    try {
      const res = await fetch(`${base}/api/health`);
      up = res.ok;
    } catch {
      /* not up yet */
    }
    if (!up) await Bun.sleep(150);
  }
  expect(up).toBe(true);
});

afterAll(() => {
  proc?.kill();
});

describe("/today routes through the cache (#1179)", () => {
  test("cold then warm /frag/today both render the fragment", async () => {
    const cold = await fetch(`${base}/frag/today`);
    expect(cold.status).toBe(200);
    const coldBody = await cold.text();
    expect(coldBody).toContain('id="today-main"');
    // Warm hit: same fragment, served from the cache.
    const warm = await fetch(`${base}/frag/today`);
    expect(warm.status).toBe(200);
    expect(await warm.text()).toBe(coldBody);
  });

  test("/today?full=1 and /api/today.json render from the same assembly", async () => {
    const full = await fetch(`${base}/today?full=1`);
    expect(full.status).toBe(200);
    expect(await full.text()).toContain("Today");
    const api = await fetch(`${base}/api/today.json`);
    expect(api.status).toBe(200);
    const body = (await api.json()) as { objectives: unknown[]; momentum: unknown[] };
    expect(Array.isArray(body.objectives)).toBe(true);
    expect(Array.isArray(body.momentum)).toBe(true);
  });

  test("the shell still serves without the full fan-out", async () => {
    const shell = await fetch(`${base}/today`);
    expect(shell.status).toBe(200);
    const html = await shell.text();
    expect(html).toContain("/frag/today");
  });
});
