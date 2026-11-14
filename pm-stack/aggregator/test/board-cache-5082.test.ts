import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawn, type Subprocess } from "bun";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { boardJsonBody } from "../src/board-json.ts";
import { DONE_STRIP_RENDER_LIMIT } from "../src/board-policy.ts";
import { TODAY_CACHE_TTL_MS, TodayCache } from "../src/today-cache.ts";
import type { BoardItem, GhItem, RefKey } from "../src/model.ts";

// #5082: the board-family assembly (boardContext / overviewContext /
// the /program source fan-out) re-ran the full live assembly on EVERY
// request — Kaneo poll, one projector /links call per card, the items
// closure — and tripped the 10 s dashboard deadline under load, with
// concurrent cold loads each starting their own assembly. server.ts
// now runs those assemblies through TodayCache instances (the #1179
// contract: 10 s TTL + single-flight coalescing, failures never
// cached), below the variant split.
//
// The unit section pins that contract with board-shaped loaders; the
// variant section pins that ?done=all and the default body are
// separate per-request derivations of ONE shared assembly (so variants
// can never cross-serve); the route section pins the wiring through a
// booted fixture-mode server.

interface BoardContextShape {
  board: BoardItem[];
  items: Map<RefKey, GhItem>;
}

const emptyContext = (): BoardContextShape => ({ board: [], items: new Map() });

describe("board assembly cache contract (#5082)", () => {
  test("concurrent cold loads share one in-flight assembly", async () => {
    let calls = 0;
    let release!: (v: BoardContextShape) => void;
    const gate = new Promise<BoardContextShape>((resolve) => {
      release = resolve;
    });
    const cache = new TodayCache<BoardContextShape>(TODAY_CACHE_TTL_MS);
    const loader = async () => {
      calls++;
      return gate;
    };
    const pending = [cache.get(loader), cache.get(loader), cache.get(loader)];
    // Let all three attach to the in-flight load before it resolves.
    await Bun.sleep(10);
    expect(calls).toBe(1);
    const ctx = emptyContext();
    release(ctx);
    const results = await Promise.all(pending);
    expect(results).toEqual([ctx, ctx, ctx]);
    expect(calls).toBe(1);
    // The coalesced result is now the warm value.
    expect(await cache.get(loader)).toBe(ctx);
    expect(calls).toBe(1);
  });

  test("a warm hit inside the TTL does not re-run the assembly; expiry serves stale + refreshes", async () => {
    let now = 1_000_000;
    let calls = 0;
    const cache = new TodayCache<BoardContextShape>(TODAY_CACHE_TTL_MS, () => now);
    const loader = async () => {
      calls++;
      return emptyContext();
    };
    const first = await cache.get(loader);
    expect(calls).toBe(1);
    now += TODAY_CACHE_TTL_MS - 1;
    expect(await cache.get(loader)).toBe(first);
    expect(calls).toBe(1);
    // Exactly at the TTL the entry is stale: the caller is served the
    // stale assembly immediately and one background refresh re-runs
    // the assembly behind it (#5082 SWR — the first-pass latency fix).
    now += 1;
    const served = await cache.get(loader);
    expect(served).toBe(first);
    expect(calls).toBe(2);
    await Bun.sleep(10);
    const reloaded = await cache.get(loader);
    expect(reloaded).not.toBe(first);
    expect(calls).toBe(2);
  });

  test("a failed assembly is never cached; the next request retries", async () => {
    // boardContext's loader fail-softs per facet, but the overview
    // loader can throw (attention) — the contract the wiring relies on
    // is that a throw releases the slot and is never served later.
    let calls = 0;
    const cache = new TodayCache<BoardContextShape>(TODAY_CACHE_TTL_MS);
    await expect(
      cache.get(async () => {
        calls++;
        throw new Error("kaneo unreachable");
      }),
    ).rejects.toThrow("kaneo unreachable");
    expect(calls).toBe(1);
    const recovered = await cache.get(async () => {
      calls++;
      return emptyContext();
    });
    expect(calls).toBe(2);
    // The successful retry is cached normally.
    expect(
      await cache.get(async () => {
        calls++;
        return emptyContext();
      }),
    ).toBe(recovered);
    expect(calls).toBe(2);
  });
});

// -- Variants: ?done=all vs the default body, one shared assembly -----

const now = Date.now();
const iso = (msAgo: number) => new Date(now - msAgo).toISOString();

const item = (ref: RefKey): GhItem => ({
  ref,
  kind: "issue",
  url: `https://github.com/${ref.replace("#", "/issues/")}`,
  title: `item ${ref}`,
  state: "open",
  labels: [],
  author: "someone",
  updatedAt: iso(0),
});

const card = (title: string, status: BoardItem["status"], links: string[], updatedAt: string): BoardItem => ({
  id: `kaneo:code/${title}`,
  title,
  repo: "developing-today/code",
  source: "kaneo",
  status,
  updatedAt,
  links: links.map((ref) => ({ ref, relation: "tracks" as const })),
});

describe("board variants derive from one shared assembly (#5082)", () => {
  test("?done=all and the default body are separate derivations of a single assembly", async () => {
    const doneCount = DONE_STRIP_RENDER_LIMIT + 50;
    const doneCards = Array.from({ length: doneCount }, (_, i) =>
      card(`done-${i}`, "done", [`developing-today/code#${1000 + i}`], iso(i * 60_000)),
    );
    const liveCards = [0, 1, 2, 3, 4].map((i) =>
      card(`live-${i}`, "in-progress", [`developing-today/code#${2000 + i}`], iso(0)),
    );
    const board = [...liveCards, ...doneCards];
    const items = new Map<RefKey, GhItem>(board.flatMap((c) => c.links.map((l) => [l.ref, item(l.ref)] as const)));

    let assemblies = 0;
    const cache = new TodayCache<BoardContextShape>(TODAY_CACHE_TTL_MS);
    const loader = async (): Promise<BoardContextShape> => {
      assemblies++;
      return { board, items };
    };

    // Both variants are derived per request from cache.get() results…
    const ctxDefault = await cache.get(loader);
    const bounded = boardJsonBody(ctxDefault.board, ctxDefault.items, { showAllDone: false });
    const ctxAll = await cache.get(loader);
    const full = boardJsonBody(ctxAll.board, ctxAll.items, { showAllDone: true });

    // …which are the SAME single assembly — nothing per-variant is
    // cached, so a variant body can never be served for another.
    expect(ctxAll).toBe(ctxDefault);
    expect(assemblies).toBe(1);

    expect(bounded.board.filter((c) => c.status === "done")).toHaveLength(DONE_STRIP_RENDER_LIMIT);
    expect(full.board.filter((c) => c.status === "done")).toHaveLength(doneCount);
    expect(bounded.board).toHaveLength(liveCards.length + DONE_STRIP_RENDER_LIMIT);
    expect(full.board).toHaveLength(liveCards.length + doneCount);
    // One link per card: each body ships exactly its visible cards' items.
    expect(bounded.items).toHaveLength(bounded.board.length);
    expect(full.items).toHaveLength(full.board.length);
    expect(bounded.board).not.toEqual(full.board);
  });
});

// -- Routes: booted fixture-mode server -------------------------------

let proc: Subprocess | undefined;
let base = "";

beforeAll(async () => {
  const dir = await mkdtemp(join(tmpdir(), "pm-board-cache-5082-"));
  const port = 7000 + Math.floor(Math.random() * 800);
  base = `http://127.0.0.1:${port}`;
  proc = spawn({
    cmd: ["bun", "run", "src/server.ts"],
    cwd: join(import.meta.dir, ".."),
    env: {
      ...Bun.env,
      PM_AGG_SOURCE: "fixtures",
      PM_AGG_PORT: String(port),
      PM_AGG_FRESHNESS_PORT: String(port + 2000),
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
      ["KANEO_ADMIN_" + "PASSWORD"]: "",
      ["GRIST_API_" + "KEY"]: "",
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

describe("board-family routes through the assembly cache (#5082)", () => {
  test("/api/board.json: warm hit is byte-identical; ?done=all never cross-serves", async () => {
    const first = await fetch(`${base}/api/board.json`);
    expect(first.status).toBe(200);
    const firstBody = await first.text();
    // Warm hit inside the TTL: the same body, served from the cache.
    const warm = await fetch(`${base}/api/board.json`);
    expect(warm.status).toBe(200);
    expect(await warm.text()).toBe(firstBody);

    const all = await fetch(`${base}/api/board.json?done=all`);
    expect(all.status).toBe(200);
    const allJson = JSON.parse(await all.text()) as { board: unknown[]; items: unknown[] };
    const defaultJson = JSON.parse(firstBody) as { board: unknown[]; items: unknown[] };
    expect(Array.isArray(allJson.board)).toBe(true);
    // The full strip is never smaller than the bounded strip.
    expect(allJson.board.length).toBeGreaterThanOrEqual(defaultJson.board.length);

    // The default variant still serves its own body after ?done=all ran.
    const again = await fetch(`${base}/api/board.json`);
    expect(await again.text()).toBe(firstBody);
  });

  test("concurrent /api/board.json requests all serve the same body", async () => {
    const bodies = await Promise.all(
      Array.from({ length: 5 }, async () => {
        const res = await fetch(`${base}/api/board.json`);
        expect(res.status).toBe(200);
        return res.text();
      }),
    );
    expect(new Set(bodies).size).toBe(1);
  });

  test("/board renders; /board?done=all renders its own variant", async () => {
    const page = await fetch(`${base}/board`);
    expect(page.status).toBe(200);
    expect(await page.text()).toContain("Cross-repo WIP board");
    const all = await fetch(`${base}/board?done=all`);
    expect(all.status).toBe(200);
    expect(await all.text()).toContain("Cross-repo WIP board");
  });

  test("/api/overview.json warm hit is byte-identical; /frag/overview renders", async () => {
    const first = await fetch(`${base}/api/overview.json`);
    expect(first.status).toBe(200);
    const firstBody = await first.text();
    const warm = await fetch(`${base}/api/overview.json`);
    expect(warm.status).toBe(200);
    expect(await warm.text()).toBe(firstBody);
    const frag = await fetch(`${base}/frag/overview`);
    expect(frag.status).toBe(200);
    expect(await frag.text()).toContain('id="overview-main"');
  });

  test("/program and /api/program.json serve through the cached fan-out", async () => {
    const page = await fetch(`${base}/program`);
    expect(page.status).toBe(200);
    const api = await fetch(`${base}/api/program.json`);
    expect(api.status).toBe(200);
    const data = JSON.parse(await api.text()) as { projects: unknown };
    expect(data).toHaveProperty("projects");
    const warm = await fetch(`${base}/api/program.json`);
    expect(warm.status).toBe(200);
  });
});
