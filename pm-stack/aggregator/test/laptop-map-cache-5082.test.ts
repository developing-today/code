import { afterAll, describe, expect, it } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { liveSource } from "../src/sources/live.ts";

// #5082 (2026-11-14): the laptop session map's failure was not
// cached, and concurrent callers each ran their own map.
// mapServerSessions walks up to ~120 sequential fetches with 10 s
// aborts, so a stalled OpenCode server cost every registry() caller
// 6-10.5 s — registry() sits on the request path of /program,
// /api/program.json and /board via commentFormOpts, and steward run
// 381 watched three sequential assemblies each pay ~10.5 s (two
// 503s). These tests pin the map-level contract: one in-flight
// attempt shared by concurrent callers, a failed attempt remembered
// briefly, a successful one cached as before.

const dir = mkdtempSync(join(tmpdir(), "agg-laptop-map-"));
afterAll(() => rmSync(dir, { recursive: true, force: true }));

interface Stub {
  url: string;
  hits: () => number;
  stop: () => void;
}

function stubServer(respond: (path: string) => Response | Promise<Response>): Stub {
  const counts = new Map<string, number>();
  const server = Bun.serve({
    port: 0,
    async fetch(req) {
      const path = new URL(req.url).pathname;
      counts.set(path, (counts.get(path) ?? 0) + 1);
      return respond(path);
    },
  });
  return {
    url: `http://127.0.0.1:${server.port}`,
    hits: () => [...counts.values()].reduce((a, b) => a + b, 0),
    stop: () => server.stop(true),
  };
}

function sourceFor(stubUrl: string) {
  const stateFile = join(dir, `opencode-state-${Math.random().toString(36).slice(2)}.json`);
  writeFileSync(stateFile, JSON.stringify({ url: stubUrl, password: "test-password" }));
  const cfg = loadConfig({
    PM_AGG_SOURCE: "live",
    PM_AGG_FEED_FILE: join(dir, "no-feed.json"),
    PM_AGG_REGISTRY_FILE: join(dir, "no-registry.json"),
    PM_AGG_OPENCODE_SERVER_STATE: stateFile,
    PM_AGG_OPENCHAMBER_STATE: join(dir, "no-openchamber-state.json"),
    PM_PROJECTOR_URL: "http://127.0.0.1:1",
  });
  return liveSource(cfg);
}

describe("laptop session map caching (#5082)", () => {
  it("remembers a failed map briefly instead of re-paying the stall per caller", async () => {
    const stub = stubServer(() => new Response("down", { status: 500 }));
    try {
      const source = sourceFor(stub.url);
      await source.registry();
      const afterFirst = stub.hits();
      expect(afterFirst).toBeGreaterThan(0);
      await source.registry();
      await source.registry();
      expect(stub.hits()).toBe(afterFirst);
    } finally {
      stub.stop();
    }
  });

  it("shares one in-flight map between concurrent callers", async () => {
    const stub = stubServer(async () => {
      await new Promise((resolve) => setTimeout(resolve, 250));
      return new Response("down", { status: 500 });
    });
    try {
      const source = sourceFor(stub.url);
      await Promise.all([source.registry(), source.registry(), source.registry(), source.registry(), source.registry()]);
      // One map attempt = one session-list fetch (the failure stops
      // the walk there), however many callers were waiting on it.
      expect(stub.hits()).toBe(1);
    } finally {
      stub.stop();
    }
  });

  it("still caches a successful map for the established 45 s window", async () => {
    const stub = stubServer((path) => {
      if (path === "/api/session") return Response.json({ data: [] });
      if (path === "/api/session/active") return Response.json({ data: {} });
      if (path === "/api/permission/request") return Response.json({ data: [] });
      return new Response("nope", { status: 404 });
    });
    try {
      const source = sourceFor(stub.url);
      await source.registry();
      const afterFirst = stub.hits();
      expect(afterFirst).toBe(3);
      await source.registry();
      expect(stub.hits()).toBe(afterFirst);
    } finally {
      stub.stop();
    }
  });
});
