// search-profile.test.ts — Slice #14060/1.
//
// Oracle: exact structured measurement fields and stable fixture
// response. This harness records; it asserts NO performance threshold.
//
// Observed baseline (this laptop, 2026-11-24, lane code-g, slice 1):
//   command: bun run scripts/search-profile.ts --query fleet --cold-runs 1 --warm-runs 3
//   server: spawned-fixtures (PM_AGG_SOURCE=fixtures + PM_AGG_DEV=1)
//   run A: JSON cold=484.3ms/1592B warm-median=1.4ms/1592B;
//          HTML cold=449.8ms/23373B warm-median=429.7ms/23373B; render-delta=428.3ms
//   run B: JSON cold=459.9ms warm-median=444.2ms (laptop under load, loadavg ~10+);
//          HTML cold=460.8ms warm-median=452.5ms; render-delta=8.3ms
//   run C: JSON cold=454.7ms warm-median=1.1ms over 5 runs;
//          HTML cold=450.2ms warm-median=334.9ms; render-delta=333.8ms
// Reading: cold floor ~450-485ms both routes (source reads + first render),
// stable across runs. Warm JSON ~1ms unloaded, spiking under laptop load.
// Warm HTML ~335-452ms even when warm JSON is ~1ms: server-side rendering
// dominates /search independent of source reads. Bytes deterministic every
// run (1592B JSON / 23373B HTML). Next slice: in-process instrumentation of
// the shared searchContext path + renderSearch breakdown; re-run the same
// command and compare.

import { describe, expect, test } from "bun:test";
import type { BoardItem, GhItem, RefKey, RoadmapRow } from "../src/model.ts";
import { searchWork } from "../src/search-view.ts";
import { median, parseProfileArgs, summarize, type MeasureRecord } from "../scripts/search-profile.ts";

describe("search-profile args — strict contract", () => {
  test("requires --query", () => {
    let code: unknown;
    const exit = process.exit;
    (process as { exit: (c: number) => void }).exit = ((c: number) => {
      code = c;
      throw new Error(`exit:${c}`);
    }) as typeof process.exit;
    try {
      expect(() => parseProfileArgs([])).toThrow("exit:2");
      expect(code).toBe(2);
    } finally {
      process.exit = exit;
    }
  });

  test("rejects unknown flags", () => {
    const exit = process.exit;
    (process as { exit: (c: number) => void }).exit = ((c: number) => {
      throw new Error("exit:2");
    }) as typeof process.exit;
    try {
      expect(() => parseProfileArgs(["--query", "x", "--bogus"])).toThrow("exit:2");
    } finally {
      process.exit = exit;
    }
  });

  test("defaults: spawn mode, 1 cold, 3 warm", () => {
    const opts = parseProfileArgs(["--query", "fleet"]);
    expect(opts).toMatchObject({
      query: "fleet",
      mode: "spawn",
      coldRuns: 1,
      warmRuns: 3,
      live: false,
      largeRegistryRecords: 0,
    });
  });

  test("large-registry mode is explicit and bounded to fixture servers", () => {
    expect(parseProfileArgs(["--query", "fleet", "--large-registry", "28479"]).largeRegistryRecords).toBe(28_479);
    const exit = process.exit;
    (process as { exit: (c: number) => void }).exit = ((c: number) => {
      throw new Error(`exit:${c}`);
    }) as typeof process.exit;
    try {
      expect(() => parseProfileArgs(["--query", "fleet", "--live", "--large-registry", "28479"])).toThrow("exit:2");
      expect(() => parseProfileArgs(["--query", "fleet", "--large-registry", "1e3"])).toThrow("exit:2");
    } finally {
      process.exit = exit;
    }
  });
});

describe("median", () => {
  test("odd, even, single", () => {
    expect(median([3, 1, 2])).toBe(2);
    expect(median([4, 1, 2, 3])).toBe(2.5);
    expect(median([7])).toBe(7);
  });
  test("empty throws", () => {
    expect(() => median([])).toThrow();
  });
});

const record = (partial: Partial<MeasureRecord> & { route: MeasureRecord["route"] }): MeasureRecord => ({
  query: "fleet",
  mode: "warm",
  run: 1,
  status: 200,
  elapsedMs: 10,
  bytes: 100,
  server: "spawned-fixtures",
  coldApproximate: false,
  at: "2026-11-24T00:00:00.000Z",
  ...partial,
});

describe("summarize", () => {
  test("warm medians and render delta", () => {
    const records: MeasureRecord[] = [
      record({ route: "/api/search.json", mode: "cold", elapsedMs: 500, bytes: 900 }),
      record({ route: "/api/search.json", run: 1, elapsedMs: 100, bytes: 900 }),
      record({ route: "/api/search.json", run: 2, elapsedMs: 120, bytes: 900 }),
      record({ route: "/search", mode: "cold", elapsedMs: 900, bytes: 950 }),
      record({ route: "/search", run: 1, elapsedMs: 200, bytes: 950 }),
      record({ route: "/search", run: 2, elapsedMs: 220, bytes: 950 }),
    ];
    const summary = summarize("fleet", "spawned-fixtures", records);
    const json = summary.routes.find((r) => r.route === "/api/search.json")!;
    const html = summary.routes.find((r) => r.route === "/search")!;
    expect(json.cold?.elapsedMs).toBe(500);
    expect(json.warmMedianElapsedMs).toBe(110);
    expect(html.warmMedianElapsedMs).toBe(210);
    expect(summary.renderDeltaMs).toBe(100);
    expect(summary.gaps.length).toBeGreaterThan(0);
  });

  test("missing warm data yields nulls, not crashes", () => {
    const summary = summarize("fleet", "external", [record({ route: "/api/search.json", mode: "cold", elapsedMs: 5 })]);
    const json = summary.routes.find((r) => r.route === "/api/search.json")!;
    expect(json.warmMedianElapsedMs).toBeNull();
    expect(summary.renderDeltaMs).toBeNull();
  });

  test("records the explicit fixture corpus size", () => {
    const summary = summarize("fleet", "spawned-fixtures", [], 28_479);
    expect(summary.largeRegistryRecords).toBe(28_479);
  });
});

describe("searchWork — stable fixture response", () => {
  const ref = "developing-today/code#14060" as RefKey;
  const board: BoardItem[] = [
    {
      id: "kaneo:pm-stack/task-9",
      title: "Bound the search read path",
      repo: "developing-today/code",
      source: "kaneo",
      status: "in-progress",
      links: [{ ref, relation: "implements" as const }],
    },
  ];
  const rows: RoadmapRow[] = [
    {
      id: 9,
      subject: "grist:program/roadmap/9",
      title: "Unrelated roadmap row",
      area: "Work",
      status: "Done",
      horizon: "October 2026",
      source: "Grist",
      notes: "nothing matching here",
      links: [],
    },
  ];
  const items = new Map<RefKey, GhItem>([
    [
      ref,
      {
        ref,
        kind: "issue",
        url: "https://example.invalid/i",
        title: "Search latency",
        state: "open",
        labels: [],
        author: "a",
        updatedAt: "2026-11-01T00:00:00Z",
        resolution: "projector",
      },
    ],
  ]);

  test("same inputs give byte-identical outputs", () => {
    const a = searchWork(board, rows, items, "search", {});
    const b = searchWork(board, rows, items, "search", {});
    expect(JSON.stringify(a)).toBe(JSON.stringify(b));
    expect(a.map((r) => r.id)).toEqual(["kaneo:pm-stack/task-9"]);
  });

  test("empty query returns no results", () => {
    expect(searchWork(board, rows, items, "   ", {})).toEqual([]);
  });
});

describe("search-profile end to end — stub server", () => {
  test("emits structured records with exact fields", async () => {
    const stub = Bun.serve({
      port: 0,
      hostname: "127.0.0.1",
      fetch: (req) => {
        const url = new URL(req.url);
        if (url.pathname === "/api/search.json") {
          return Response.json({ query: "fleet", total: 1, results: [] });
        }
        return new Response("<html>search</html>", { headers: { "content-type": "text/html" } });
      },
    });
    try {
      const proc = Bun.spawn(
        [
          "bun",
          "run",
          "scripts/search-profile.ts",
          "--query",
          "fleet",
          "--mode",
          "base-url",
          "--base-url",
          `http://127.0.0.1:${stub.port}`,
          "--warm-runs",
          "2",
        ],
        { cwd: new URL(".", import.meta.url).pathname.replace(/\/test\/$/, ""), stdout: "pipe", stderr: "ignore" },
      );
      const out = await new Response(proc.stdout).text();
      const code = await proc.exited;
      expect(code).toBe(0);
      const parsed = JSON.parse(out) as { records: MeasureRecord[]; summary: ReturnType<typeof summarize> };
      expect(parsed.records.length).toBe(2 * (1 + 2));
      for (const r of parsed.records) {
        expect(["/api/search.json", "/search"]).toContain(r.route);
        expect(["cold", "warm"]).toContain(r.mode);
        expect(r.status).toBe(200);
        expect(typeof r.elapsedMs).toBe("number");
        expect(r.elapsedMs).toBeGreaterThanOrEqual(0);
        expect(Number.isInteger(r.bytes)).toBe(true);
        expect(r.bytes).toBeGreaterThan(0);
        expect(r.server).toBe("external");
        expect(typeof r.at).toBe("string");
      }
      expect(parsed.summary.renderDeltaMs).not.toBeNull();
    } finally {
      stub.stop(true);
    }
  }, 60_000);
});
