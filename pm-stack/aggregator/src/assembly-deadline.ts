const ASSEMBLY_DEADLINE_MS = 10_000;
const MAX_STALE_AGE_MS = 5 * 60_000;
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
}

interface DeadlineOptions {
  timeoutMs?: number;
  maxStaleAgeMs?: number;
  maxCachedResponseBytes?: number;
  maxCacheEntries?: number;
  now?: () => number;
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
  const lastGood = new Map<string, CachedResponse>();
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
    lastGood.set(key, { body, headers, status: response.status, storedAt: now() });
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
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<{ kind: "timeout" }>((resolve) => {
      timer = setTimeout(() => resolve({ kind: "timeout" }), timeoutMs);
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
    const cached = lastGood.get(key);
    if (cached && now() - cached.storedAt <= maxStaleAgeMs) {
      return staleResponse(url.pathname, cached, timeoutMs);
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

function staleResponse(path: string, cached: CachedResponse, timeoutMs: number): Response {
  const headers = new Headers(cached.headers);
  headers.set("x-aggregator-stale", "true");
  headers.set("cache-control", "no-store");
  const text = new TextDecoder().decode(cached.body);
  const notice = `Showing the last successful dashboard response; the latest assembly timed out after ${formatDuration(timeoutMs)}.`;

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

function formatDuration(timeoutMs: number): string {
  return timeoutMs % 1000 === 0 ? `${timeoutMs / 1000} seconds` : `${timeoutMs} ms`;
}
