// search-profile.ts — Slice #14060/1: repeatable search-path profiler.
//
// Measures the two slow search routes from #14060 (GET /api/search.json
// and GET /search for the same query), distinguishing cold and warm
// behavior and reporting latency plus response bytes. No performance
// threshold is asserted anywhere: the harness records, it never judges.
//
// Two modes:
//   spawn    (default) boots its own aggregator server(s) per measurement
//            phase, so cold means a genuinely cold process (cold caches)
//            and warm means steady state on one process. Fixture source
//            (PM_AGG_SOURCE=fixtures + PM_AGG_DEV=1) is the default: fully
//            hermetic, stable fixture response, repeatable anywhere.
//            --live switches the spawned server to the live source.
//            --large-registry N injects a deterministic N-record fixture
//            registry for representative scan-bound measurements.
//   base-url profiles a running server (e.g. the laptop production
//            aggregator). Cold there is the first request observed and is
//            only an approximation — a truly cold read needs a restart,
//            which this mode never triggers. Documented in the output.
//
// Stage attribution (what the seam permits):
//   Over HTTP only per-route totals are observable. The summary reports
//   the warm-median HTML-minus-JSON delta as the render+transfer cost of
//   the /search page over the shared searchContext path. Splitting the
//   shared path itself (source.board/roadmap/registry/dns/usage/
//   orchestration reads vs items resolution vs searchWork vs JSON
//   serialization) is NOT observable over HTTP and needs in-process
//   instrumentation — recorded as the remaining gap for the next slice.
//
// ONLY this file and test/search-profile.test.ts belong to the slice.

import { spawn, type Subprocess } from "bun";

export const ROUTES = ["/api/search.json", "/search"] as const;
export type Route = (typeof ROUTES)[number];
export type Mode = "cold" | "warm";
export type ServerKind = "spawned-fixtures" | "spawned-live" | "external";

export interface MeasureRecord {
  route: Route;
  query: string;
  mode: Mode;
  /** 1-based within its mode. */
  run: number;
  status: number;
  /** Whole-request elapsed wall time, milliseconds. */
  elapsedMs: number;
  /** Response body bytes as received. */
  bytes: number;
  server: ServerKind;
  /** Cold in base-url mode is first-observed, not process-cold. */
  coldApproximate: boolean;
  at: string;
}

export interface ProfileOptions {
  query: string;
  mode: "spawn" | "base-url";
  baseUrl: string;
  live: boolean;
  largeRegistryRecords: number;
  coldRuns: number;
  warmRuns: number;
  port: number;
  timeoutMs: number;
}

export const USAGE =
  "usage: search-profile.ts --query <q> [--mode spawn|base-url] [--base-url <url>] [--live] " +
  "[--large-registry <n>] [--cold-runs <n>] [--warm-runs <n>] [--port <n>] [--timeout-ms <ms>]";

export function usageError(message: string): never {
  console.error(`search-profile: ${message}`);
  console.error(USAGE);
  process.exit(2);
}

const VALUE_FLAGS = new Set([
  "--query",
  "--mode",
  "--base-url",
  "--large-registry",
  "--cold-runs",
  "--warm-runs",
  "--port",
  "--timeout-ms",
]);
const BOOL_FLAGS = new Set(["--live"]);

function parsePositiveInt(raw: string, flag: string): number {
  const n = Number(raw);
  if (!Number.isInteger(n) || n < 0) usageError(`${flag} must be a non-negative integer, got ${JSON.stringify(raw)}`);
  return n;
}

export function parseProfileArgs(argv: string[]): ProfileOptions {
  let query: string | undefined;
  let mode: "spawn" | "base-url" = "spawn";
  let baseUrl = "";
  let live = false;
  let largeRegistryRecords = 0;
  let coldRuns = 1;
  let warmRuns = 3;
  let port = 0;
  let timeoutMs = 120_000;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (BOOL_FLAGS.has(arg)) {
      live = true;
      continue;
    }
    if (!VALUE_FLAGS.has(arg)) usageError(`unknown argument ${JSON.stringify(arg)}`);
    const value = argv[++i];
    if (value === undefined || value.startsWith("--")) usageError(`${arg} needs a value`);
    switch (arg) {
      case "--query":
        query = value;
        break;
      case "--mode":
        if (value !== "spawn" && value !== "base-url") usageError(`--mode must be spawn|base-url`);
        mode = value;
        break;
      case "--base-url":
        baseUrl = value.replace(/\/$/, "");
        break;
      case "--large-registry":
        if (!/^\d+$/.test(value)) usageError("--large-registry must be decimal digits");
        largeRegistryRecords = Number.parseInt(value, 10);
        if (largeRegistryRecords < 1 || largeRegistryRecords > 100_000) {
          usageError("--large-registry must be between 1 and 100000");
        }
        break;
      case "--cold-runs":
        coldRuns = parsePositiveInt(value, arg);
        break;
      case "--warm-runs":
        warmRuns = parsePositiveInt(value, arg);
        break;
      case "--port":
        port = parsePositiveInt(value, arg);
        break;
      case "--timeout-ms":
        timeoutMs = parsePositiveInt(value, arg);
        break;
    }
  }
  if (query === undefined || query.trim() === "") usageError("--query <q> is required");
  if (mode === "base-url" && baseUrl === "") usageError("--mode base-url needs --base-url <url>");
  if (largeRegistryRecords > 0 && (mode !== "spawn" || live)) {
    usageError("--large-registry requires spawned fixture mode");
  }
  if (coldRuns === 0 && warmRuns === 0) usageError("at least one of --cold-runs/--warm-runs must be > 0");
  return { query: query.trim(), mode, baseUrl, live, largeRegistryRecords, coldRuns, warmRuns, port, timeoutMs };
}

export function median(values: number[]): number {
  if (values.length === 0) throw new Error("median of no values");
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

export interface RouteSummary {
  route: Route;
  cold: { elapsedMs: number; bytes: number } | null;
  warmRuns: number;
  warmMedianElapsedMs: number | null;
  warmMedianBytes: number | null;
}

export interface ProfileSummary {
  query: string;
  server: ServerKind;
  /** Synthetic registry size, only for explicit large-fixture runs. */
  largeRegistryRecords: number | null;
  coldApproximate: boolean;
  routes: RouteSummary[];
  /** Warm-median HTML total minus warm-median JSON total: the /search
   * page render+transfer cost over the shared searchContext path.
   * Null unless both routes have a warm median. */
  renderDeltaMs: number | null;
  gaps: string[];
}

export const STAGE_GAP =
  "source/board,roadmap,registry,dns,usage,orchestration reads vs items resolution vs searchWork vs " +
  "serialization are not separable over HTTP; splitting the shared searchContext path needs in-process instrumentation.";

export function summarize(
  query: string,
  server: ServerKind,
  records: MeasureRecord[],
  largeRegistryRecords = 0,
): ProfileSummary {
  const coldApproximate = records.some((r) => r.coldApproximate);
  const routes = ROUTES.map((route) => {
    const forRoute = records.filter((r) => r.route === route);
    const cold = forRoute.find((r) => r.mode === "cold") ?? null;
    const warm = forRoute.filter((r) => r.mode === "warm").map((r) => r.elapsedMs);
    const warmBytes = forRoute.filter((r) => r.mode === "warm").map((r) => r.bytes);
    return {
      route,
      cold: cold ? { elapsedMs: cold.elapsedMs, bytes: cold.bytes } : null,
      warmRuns: warm.length,
      warmMedianElapsedMs: warm.length > 0 ? median(warm) : null,
      warmMedianBytes: warmBytes.length > 0 ? median(warmBytes) : null,
    };
  });
  const json = routes.find((r) => r.route === "/api/search.json");
  const html = routes.find((r) => r.route === "/search");
  const renderDeltaMs =
    json?.warmMedianElapsedMs != null && html?.warmMedianElapsedMs != null
      ? html.warmMedianElapsedMs - json.warmMedianElapsedMs
      : null;
  return {
    query,
    server,
    largeRegistryRecords: largeRegistryRecords || null,
    coldApproximate,
    routes,
    renderDeltaMs,
    gaps: [STAGE_GAP],
  };
}

async function grabFreePort(): Promise<number> {
  const stub = Bun.serve({ port: 0, hostname: "127.0.0.1", fetch: () => new Response("ok") });
  const port = stub.port;
  stub.stop(true);
  return port;
}

interface Spawned {
  proc: Subprocess;
  base: string;
}

async function spawnServer(
  port: number,
  live: boolean,
  timeoutMs: number,
  largeRegistryRecords: number,
): Promise<Spawned> {
  const base = `http://127.0.0.1:${port}`;
  const env: Record<string, string> = {
    ...process.env,
    PM_AGG_PORT: String(port),
    PM_AGG_FRESHNESS_PORT: String(port + 1),
  };
  if (live) {
    env.PM_AGG_SOURCE = "live";
  } else {
    env.PM_AGG_SOURCE = "fixtures";
    env.PM_AGG_DEV = "1";
    if (largeRegistryRecords > 0) env.PM_AGG_LARGE_REGISTRY = String(largeRegistryRecords);
  }
  const proc = Bun.spawn(["bun", "run", "src/server.ts"], {
    cwd: new URL("..", import.meta.url).pathname,
    env,
    stdout: "ignore",
    stderr: "ignore",
  });
  const deadline = Date.now() + Math.min(timeoutMs, 60_000);
  for (;;) {
    try {
      const res = await fetch(`${base}/api/health`, { signal: AbortSignal.timeout(5_000) });
      await res.arrayBuffer();
      if (res.status < 500) break;
    } catch {
      /* not up yet */
    }
    if (proc.exitCode !== null) throw new Error(`aggregator server exited during boot (code ${proc.exitCode})`);
    if (Date.now() > deadline) {
      proc.kill();
      throw new Error("aggregator server did not answer /api/health in time");
    }
    await Bun.sleep(250);
  }
  return { proc, base };
}

async function measureRoute(
  base: string,
  route: Route,
  query: string,
  mode: Mode,
  run: number,
  server: ServerKind,
  coldApproximate: boolean,
  timeoutMs: number,
): Promise<MeasureRecord> {
  const url = `${base}${route}?q=${encodeURIComponent(query)}`;
  const started = performance.now();
  const res = await fetch(url, { signal: AbortSignal.timeout(timeoutMs) });
  const buf = await res.arrayBuffer();
  const elapsedMs = performance.now() - started;
  return {
    route,
    query,
    mode,
    run,
    status: res.status,
    elapsedMs: Math.round(elapsedMs * 10) / 10,
    bytes: buf.byteLength,
    server,
    coldApproximate,
    at: new Date().toISOString(),
  };
}

async function measurePhase(
  base: string,
  opts: ProfileOptions,
  mode: Mode,
  runs: number,
  server: ServerKind,
  coldApproximate: boolean,
  out: MeasureRecord[],
): Promise<void> {
  for (let run = 1; run <= runs; run++) {
    for (const route of ROUTES) {
      out.push(await measureRoute(base, route, opts.query, mode, run, server, coldApproximate, opts.timeoutMs));
    }
  }
}

function humanSummary(summary: ProfileSummary): string {
  const lines = [
    `search-profile: query=${JSON.stringify(summary.query)} server=${summary.server}${
      summary.coldApproximate ? " (cold is first-observed, approximate)" : ""
    }${summary.largeRegistryRecords ? ` registry-records=${summary.largeRegistryRecords}` : ""}`,
  ];
  for (const r of summary.routes) {
    lines.push(
      `  ${r.route}: cold=${r.cold ? `${r.cold.elapsedMs}ms/${r.cold.bytes}B` : "n/a"} ` +
        `warm-median=${r.warmMedianElapsedMs ?? "n/a"}ms/${r.warmMedianBytes ?? "n/a"}B over ${r.warmRuns} runs`,
    );
  }
  lines.push(`  render-delta-warm-median=${summary.renderDeltaMs ?? "n/a"}ms (HTML total minus JSON total)`);
  lines.push(`  gap: ${STAGE_GAP}`);
  return lines.join("\n");
}

async function main(): Promise<void> {
  const opts = parseProfileArgs(process.argv.slice(2));
  const records: MeasureRecord[] = [];
  if (opts.mode === "base-url") {
    const server: ServerKind = "external";
    // One unrecorded warmup would destroy the only cold observation in
    // this mode, so the first request per route IS the cold record.
    await measurePhase(opts.baseUrl, opts, "cold", Math.min(opts.coldRuns, 1), server, true, records);
    await measurePhase(opts.baseUrl, opts, "warm", opts.warmRuns, server, true, records);
    if (opts.coldRuns > 1) {
      console.error("search-profile: base-url mode records a single cold observation; --cold-runs > 1 ignored");
    }
  } else {
    const server: ServerKind = opts.live ? "spawned-live" : "spawned-fixtures";
    for (let c = 0; c < opts.coldRuns; c++) {
      const port = opts.port || (await grabFreePort());
      const { proc, base } = await spawnServer(port, opts.live, opts.timeoutMs, opts.largeRegistryRecords);
      try {
        await measurePhase(base, opts, "cold", 1, server, false, records);
      } finally {
        proc.kill();
        await proc.exited.catch(() => {});
      }
    }
    if (opts.warmRuns > 0) {
      const port = opts.port || (await grabFreePort());
      const { proc, base } = await spawnServer(port, opts.live, opts.timeoutMs, opts.largeRegistryRecords);
      try {
        // Unrecorded warmup: steady state, not boot transient.
        await measurePhase(base, opts, "warm", 1, server, false, []);
        await measurePhase(base, opts, "warm", opts.warmRuns, server, false, records);
      } finally {
        proc.kill();
        await proc.exited.catch(() => {});
      }
    }
  }
  const summary = summarize(
    opts.query,
    opts.mode === "base-url" ? "external" : opts.live ? "spawned-live" : "spawned-fixtures",
    records,
    opts.largeRegistryRecords,
  );
  console.log(JSON.stringify({ records, summary }, null, 2));
  console.error(humanSummary(summary));
}

if (import.meta.main) {
  await main();
}
