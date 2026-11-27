import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { join } from "node:path";

const ASSEMBLY_DEADLINE_MS = 10_000;
// #5082 (2026-11-14): the last-good staleness cap was 5 minutes, but
// the bounded routes' steadiest reader is an hourly watch — at that
// cadence the last good response is routinely ~1 h old when an
// assembly blows the deadline, so the cap converted every slow
// first pass into a hard 503 instead of the marked-stale serve this
// fallback exists for (steward runs 378/379). A stale response is
// explicitly marked (x-aggregator-stale header; stale/staleReason in
// JSON bodies), so a 30-minute cap stays honest while covering the
// observed failure windows; anything older still 503s.
const MAX_STALE_AGE_MS = 30 * 60_000;
// #10383: a restart empties the in-memory last-good, so a cold first
// request whose assembly outlived the deadline had nothing to serve and
// 503'd or timed out (steward runs 387/393). Each last-good response is
// now also written to disk, and a key with nothing in memory reads its
// disk copy once. A copy that predates this process may be older than
// the ordinary stale cap — the restart is usually a deploy after an
// hourly watch — so it gets a longer cap of its own. It is marked stale
// like any other fallback, with its age in the notice.
const MAX_DISK_STALE_AGE_MS = 6 * 60 * 60_000;
// #10383 residual: the disk copy was only read after the full assembly
// deadline expired, so a cold first request still stalled the whole
// deadline (10 s) before it could serve last-good — and a client with a
// shorter timeout still saw a failure. On a cold request the disk copy
// is now loaded up front and the fresh assembly only gets this short
// grace to beat it; past the grace the disk copy is served at once
// while the assembly keeps running in the background and seeds the
// cache on settle. A fresh assembly that finishes inside the grace
// still wins, unchanged.
const COLD_STALE_GRACE_MS = 1_000;
const MAX_CACHED_RESPONSE_BYTES = 2 * 1024 * 1024;
const MAX_CACHE_ENTRIES = 128;

/** Routes whose complete data assembly can outlive the dashboard's request window. */
export const BOUNDED_DASHBOARD_PATHS: ReadonlySet<string> = new Set([
  "/board",
  "/api/board.json",
  "/api/overview.json",
  "/frag/overview",
  "/program",
  // #9781: the JSON twin joins its HTML sibling — pool-quota-steward
  // run 369 measured an unbounded 17.83 s first pass on this route
  // while every bounded route at least answered stale-or-503.
  "/api/program.json",
]);

interface CachedResponse {
  body: Uint8Array;
  headers: Headers;
  status: number;
  storedAt: number;
  /** Read back from disk after a restart (#10383). */
  fromDisk?: boolean;
}

/** The on-disk form of one last-good response (#10383). */
interface PersistedResponse {
  key: string;
  status: number;
  headers: [string, string][];
  storedAt: number;
  body: string;
}

interface DeadlineOptions {
  timeoutMs?: number;
  maxStaleAgeMs?: number;
  maxCachedResponseBytes?: number;
  maxCacheEntries?: number;
  now?: () => number;
  /** #10383: keep each last-good response here too, for the first
   *  request after a restart. Unset keeps the cache in memory only. */
  persistDir?: string;
  maxDiskStaleAgeMs?: number;
  /** #10383: how long a cold request (no in-memory last-good, disk
   *  copy available) waits for the fresh assembly before serving the
   *  disk copy. Capped by timeoutMs. */
  coldStaleGraceMs?: number;
}

/** A filesystem-safe file name for one bounded key. */
function persistName(key: string): string {
  return `${encodeURIComponent(key).replace(/%/g, "_").slice(0, 200)}.json`;
}

/**
 * Bounds whole-route assembly rather than just individual network calls.
 * Late successful work seeds the same URL's last-good cache for the next
 * request; a timed-out request itself always receives bytes promptly.
 */
export function createDashboardResponseDeadline(options: DeadlineOptions = {}) {
  const timeoutMs = options.timeoutMs ?? ASSEMBLY_DEADLINE_MS;
  const maxStaleAgeMs = options.maxStaleAgeMs ?? MAX_STALE_AGE_MS;
  const maxCachedResponseBytes = options.maxCachedResponseBytes ?? MAX_CACHED_RESPONSE_BYTES;
  const maxCacheEntries = options.maxCacheEntries ?? MAX_CACHE_ENTRIES;
  const now = options.now ?? Date.now;
  const persistDir = options.persistDir;
  const maxDiskStaleAgeMs = options.maxDiskStaleAgeMs ?? MAX_DISK_STALE_AGE_MS;
  const coldStaleGraceMs = options.coldStaleGraceMs ?? COLD_STALE_GRACE_MS;
  const lastGood = new Map<string, CachedResponse>();
  // Keys whose disk copy was already looked for in this process: the
  // disk is read at most once per key, on its first fallback.
  const diskChecked = new Set<string>();
  let writeSeq = 0;

  async function persist(key: string, entry: CachedResponse): Promise<void> {
    if (!persistDir) return;
    const record: PersistedResponse = {
      key,
      status: entry.status,
      headers: headerPairs(entry.headers),
      storedAt: entry.storedAt,
      body: Buffer.from(entry.body).toString("base64"),
    };
    await mkdir(persistDir, { recursive: true });
    const file = join(persistDir, persistName(key));
    const tmp = `${file}.tmp-${process.pid}-${++writeSeq}`;
    await writeFile(tmp, JSON.stringify(record));
    await rename(tmp, file);
  }

  async function fromDisk(key: string): Promise<CachedResponse | undefined> {
    if (!persistDir || diskChecked.has(key)) return undefined;
    diskChecked.add(key);
    try {
      const record = JSON.parse(await readFile(join(persistDir, persistName(key)), "utf8")) as PersistedResponse;
      if (record.key !== key || typeof record.storedAt !== "number" || typeof record.body !== "string")
        return undefined;
      const body = new Uint8Array(Buffer.from(record.body, "base64"));
      if (body.byteLength === 0 || body.byteLength > maxCachedResponseBytes) return undefined;
      return {
        body,
        headers: new Headers(record.headers),
        status: record.status,
        storedAt: record.storedAt,
        fromDisk: true,
      };
    } catch {
      return undefined;
    }
  }
  // Single-flight (#7065): one in-flight assemble() run per bounded key,
  // shared by every overlapping waiter. The entry lives until the run
  // settles AND its last-good seeding has finished, so a request that
  // arrives after one waiter's deadline — while the run is still going —
  // joins the same run instead of starting a second full assembly.
  const inFlight = new Map<string, Promise<Response>>();

  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) throw new Error("timeoutMs must be positive");

  async function remember(key: string, response: Response): Promise<void> {
    if (response.status < 200 || response.status >= 300) return;
    const body = new Uint8Array(await response.clone().arrayBuffer());
    if (body.byteLength === 0 || body.byteLength > maxCachedResponseBytes) return;

    const headers = new Headers(response.headers);
    headers.delete("content-length");
    headers.delete("content-encoding");
    headers.delete("etag");
    lastGood.delete(key);
    const entry: CachedResponse = { body, headers, status: response.status, storedAt: now() };
    lastGood.set(key, entry);
    diskChecked.add(key);
    // Best-effort: a failed write leaves the in-memory cache as it is.
    await persist(key, entry).catch(() => {});
    while (lastGood.size > maxCacheEntries) {
      const oldest = lastGood.keys().next().value;
      if (oldest === undefined) break;
      lastGood.delete(oldest);
    }
  }

  return async function dashboardResponseWithinDeadline(
    request: Request,
    assemble: () => Promise<Response>,
  ): Promise<Response> {
    const url = new URL(request.url);
    if (request.method !== "GET" || !BOUNDED_DASHBOARD_PATHS.has(url.pathname)) return assemble();

    const key = `${url.pathname}${url.search}`;
    let pending = inFlight.get(key);
    if (!pending) {
      // The shared run owns last-good seeding exactly once, on settle,
      // whether or not any individual waiter is still within its deadline.
      // A seeding failure must not turn a successful assembly into an
      // error for the waiters, so it is swallowed here (the cache simply
      // stays as it was). A rejected assemble() still rejects the shared
      // promise, so every waiter observes the ordinary error path and the
      // cleared entry lets the next request start a fresh run.
      pending = (async () => {
        const response = await Promise.resolve().then(assemble);
        try {
          await remember(key, response);
        } catch {
          // Cache seeding is best-effort; the response itself is unaffected.
        }
        return response;
      })();
      inFlight.set(key, pending);
      const clearInFlight = () => {
        if (inFlight.get(key) === pending) inFlight.delete(key);
      };
      void pending.then(clearInFlight, clearInFlight);
    }
    // Cold path (#10383): with nothing in memory, load the disk copy
    // before waiting. The assembly above is already running, so this
    // read overlaps it. A disk copy inside its cap shortens this
    // waiter's wait to the cold grace; a missing/expired/corrupt copy
    // leaves the ordinary deadline behaviour untouched.
    let coldCached: CachedResponse | undefined;
    if (!lastGood.has(key)) {
      const disk = await fromDisk(key);
      // A run that settled while the disk was read wins over the copy.
      if (disk && !lastGood.has(key)) lastGood.set(key, disk);
      const loaded = lastGood.get(key);
      if (loaded?.fromDisk && now() - loaded.storedAt <= maxDiskStaleAgeMs) coldCached = loaded;
    }
    const waitMs = coldCached ? Math.min(coldStaleGraceMs, timeoutMs) : timeoutMs;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<{ kind: "timeout" }>((resolve) => {
      timer = setTimeout(() => resolve({ kind: "timeout" }), waitMs);
    });

    let result: { kind: "timeout" } | { kind: "response"; response: Response };
    try {
      result = await Promise.race([pending.then((response) => ({ kind: "response" as const, response })), timeout]);
    } finally {
      if (timer !== undefined) clearTimeout(timer);
    }

    if (result.kind === "response") {
      // Each waiter gets its own clone: the shared Response body can only
      // be consumed once, but every coalesced waiter must receive the full
      // representation. The shared run already seeded the last-good cache.
      return result.response.clone();
    }

    // This waiter is out of time, but the shared run keeps going: it seeds
    // the last-good cache on settle (once, in the shared wrapper above),
    // improving the next request without delaying this bounded fallback.
    let cached = lastGood.get(key);
    if (!cached) {
      cached = await fromDisk(key);
      // A run that settled while the disk was read wins over the copy.
      if (cached && !lastGood.has(key)) lastGood.set(key, cached);
      cached = lastGood.get(key);
    }
    const cap = cached?.fromDisk ? maxDiskStaleAgeMs : maxStaleAgeMs;
    if (cached && now() - cached.storedAt <= cap) {
      // Cold early serve: the assembly has not timed out, it is still
      // running past the cold grace — say so, not "timed out".
      if (coldCached && cached === coldCached && waitMs < timeoutMs) {
        return staleResponse(url.pathname, cached, timeoutMs, now(), true);
      }
      return staleResponse(url.pathname, cached, timeoutMs, now());
    }
    if (cached) lastGood.delete(key);
    return timeoutResponse(url.pathname, timeoutMs);
  };
}

export const dashboardResponseWithinDeadline = createDashboardResponseDeadline();

function timeoutResponse(path: string, timeoutMs: number): Response {
  const message = `Dashboard assembly timed out after ${formatDuration(timeoutMs)}. Please retry shortly.`;
  if (path === "/frag/overview") {
    // Inner patch into the existing #overview-main shell: the fragment must
    // not carry the shell's id, or the DOM ends up with two #overview-main
    // elements and the next outer patch morphs a detached node (Codex P2 on
    // PR #6597). The shell keeps its data-frag refresh hooks, so recovery is
    // the next ordinary outer patch.
    const fragment = `<div role="alert" class="aggregator-error">${message}</div>`;
    return new Response(
      `event: datastar-patch-elements\ndata: mode inner\ndata: selector #overview-main\ndata: elements ${fragment}\n\n`,
      { status: 200, headers: { "content-type": "text/event-stream; charset=utf-8", "cache-control": "no-cache" } },
    );
  }
  if (path.startsWith("/api/")) {
    return Response.json({ error: message, stale: false, timeoutMs }, { status: 503 });
  }
  return new Response(
    `<!doctype html><html><body><main role="alert"><h1>Dashboard temporarily unavailable</h1><p>${message}</p></main></body></html>`,
    { status: 503, headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store" } },
  );
}

function staleResponse(path: string, cached: CachedResponse, timeoutMs: number, at: number, stillRunning = false): Response {
  const headers = new Headers(cached.headers);
  headers.set("x-aggregator-stale", "true");
  headers.set("cache-control", "no-store");
  const text = new TextDecoder().decode(cached.body);
  const age = cached.fromDisk
    ? ` It was saved before the dashboard restarted, ${formatAge(at - cached.storedAt)} ago.`
    : "";
  const notice = stillRunning
    ? `Showing the last successful dashboard response; the latest assembly is still running.${age}`
    : `Showing the last successful dashboard response; the latest assembly timed out after ${formatDuration(timeoutMs)}.${age}`;

  if ((headers.get("content-type") ?? "").includes("application/json")) {
    let body: unknown;
    try {
      body = JSON.parse(text);
    } catch {
      body = { data: text };
    }
    const marked =
      body !== null && typeof body === "object" && !Array.isArray(body)
        ? { ...body, stale: true, staleReason: notice }
        : { data: body, stale: true, staleReason: notice };
    return new Response(JSON.stringify(marked, null, 2), { status: cached.status, headers });
  }

  if ((headers.get("content-type") ?? "").includes("text/event-stream")) {
    const fragment = `<div role="status" class="aggregator-stale">${notice}</div>`;
    const event = `event: datastar-patch-elements\ndata: mode append\ndata: selector #overview-main\ndata: elements ${fragment}\n\n`;
    return new Response(`${text}${event}`, { status: cached.status, headers });
  }

  const banner = `<div role="status" class="aggregator-stale">${notice}</div>`;
  const bodyStart = /<body\b[^>]*>/i.exec(text);
  const marked = bodyStart
    ? `${text.slice(0, bodyStart.index + bodyStart[0].length)}${banner}${text.slice(bodyStart.index + bodyStart[0].length)}`
    : `${banner}${text}`;
  return new Response(marked, { status: cached.status, headers });
}

function formatAge(ms: number): string {
  const minutes = Math.max(0, Math.round(ms / 60_000));
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  return `${hours} h ${minutes % 60} min`;
}

function formatDuration(timeoutMs: number): string {
  return timeoutMs % 1000 === 0 ? `${timeoutMs / 1000} seconds` : `${timeoutMs} ms`;
}

function headerPairs(headers: Headers): [string, string][] {
  const pairs: [string, string][] = [];
  headers.forEach((value, name) => pairs.push([name, value]));
  return pairs;
}
