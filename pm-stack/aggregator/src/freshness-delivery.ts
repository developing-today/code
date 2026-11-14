import type { FreshnessNotice, FreshnessSource } from "./freshness.ts";

/**
 * #3978: the browser delivery layer for the #3977 freshness journal.
 *
 * The journal (src/freshness.ts) records *that* something changed and which
 * scopes it affects. It deliberately carries no source payload; a client that
 * receives a notice rereads the canonical `/api/*.json` endpoint. This module
 * turns the journal into the two browser-facing surfaces:
 *
 *   GET /freshness/stream            long-lived SSE (EventSource)
 *   GET /api/freshness-events.json   the machine-first twin, same envelope
 *
 * Both surfaces serve the same notice shape, cursor, and reset signal, so the
 * HTML and JSON consumers can never disagree about what is invalid.
 */

/** Page name -> the journal scopes whose notices invalidate that page.
 *  The mapping is server-owned: a client picks a page, never a scope set.
 *  Keys are page ids from src/pages.ts. */
export const FRESHNESS_PAGE_SCOPES: Readonly<Record<string, readonly string[]>> = {
  overview: ["board", "feed", "items", "orchestration"],
  today: ["board", "feed", "items", "orchestration"],
  digest: ["board", "feed", "items", "orchestration"],
  attention: ["board", "feed", "items"],
  board: ["board", "items", "orchestration"],
  roadmap: ["items", "orchestration"],
  program: ["board", "items", "orchestration"],
  queue: ["board", "items"],
  events: ["board", "feed", "items", "orchestration"],
  health: ["board", "feed", "items"],
  quota: ["feed"],
  "comments-queue": ["items"],
};

/** Journal scopes the aggregator's publishers emit today: the fleet-feed
 *  snapshot (`feed`), the Kaneo board poll (`board`), and per-item GitHub
 *  refs (`items` family). `orchestration` is reserved for the orchestration
 *  publisher. */
const PAGE_SCOPE_NAMES = new Set(["board", "feed", "items", "orchestration"]);
const ITEM_SCOPE = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+#[1-9][0-9]*$/;
const MAX_SCOPE_VALUES = 32;
const DEFAULT_HEARTBEAT_MS = 25_000;
const DEFAULT_POLL_MS = 2_000;
const MAX_AFTER = Number.MAX_SAFE_INTEGER;

export interface FreshnessDeliveryScope {
  /** The page the connection belongs to, when the client named one. */
  page: string | null;
  /** Scopes this connection may see; null means "every allow-listed scope". */
  scopes: ReadonlySet<string> | null;
}

export type DeliveryScopeResult =
  | { ok: true; scope: FreshnessDeliveryScope }
  | { ok: false; status: number; error: string };

/** Boolean shape of the reader this module needs from FreshnessStore. */
export interface FreshnessJournalReader {
  listAfter(
    after: number,
    scope?: string,
  ): Promise<{ cursor: number; resetRequired: boolean; events: FreshnessNotice[] }>;
}

export interface SourceHealth {
  notices: number;
  lastObservedAt: string | null;
  state: "live" | "stale" | "silent";
}

export interface FreshnessEnvelope {
  schemaVersion: 1;
  cursor: number;
  resetRequired: boolean;
  generatedAt: string;
  scopes: string[];
  events: FreshnessNotice[];
  sources: Record<FreshnessSource, SourceHealth>;
}

const SOURCE_NAMES: FreshnessSource[] = ["github", "kaneo", "fleet-feed", "orchestration", "reconcile"];

/** Pages whose body is a single Datastar fragment container (#1444). A notice
 *  that invalidates one of these patches that container by re-mounting it with
 *  its own `data-on-load`, so the existing `/frag/*` route performs the
 *  canonical re-render. Pages without a fragment container get the notice
 *  alone; their client decides when to reread. */
export const FRESHNESS_PAGE_FRAGMENTS: Readonly<Record<string, { id: string; path: string }>> = {
  overview: { id: "overview-main", path: "/frag/overview" },
  board: { id: "board-main", path: "/frag/board" },
  today: { id: "today-main", path: "/frag/today" },
  queue: { id: "queue-main", path: "/frag/queue" },
};

/** One Datastar patch-elements frame in the format the dashboard already
 *  emits (src/server.ts `ssePatch`): outer mode replaces the container with
 *  itself plus its own load/refresh hooks, which re-issues the fragment GET. */
export function freshnessPatchFrame(page: string): string | null {
  const fragment = FRESHNESS_PAGE_FRAGMENTS[page];
  if (!fragment) return null;
  const element = `<div id="${fragment.id}" data-frag data-on-load="@get('${fragment.path}')" data-on:dashrefresh__window="@get('${fragment.path}')"><div class="skel">Refreshing…</div></div>`;
  const lines = element
    .split("\n")
    .map((line) => `data: elements ${line}`)
    .join("\n");
  return `event: datastar-patch-elements\ndata: mode outer\ndata: selector #${fragment.id}\n${lines}\n\n`;
}

/** A source is "live" while a retained notice is younger than the read
 *  cadence the dashboard already tolerates (5 minutes), "stale" inside the
 *  24-hour retention window, and "silent" when it has no retained notice. */
const LIVE_WINDOW_MS = 5 * 60_000;
const STALE_WINDOW_MS = 24 * 60 * 60_000;

export function deliveryScopeFromUrl(url: URL): DeliveryScopeResult {
  const rawPage = url.searchParams.get("page");
  const page = rawPage === null || rawPage === "" ? null : rawPage;
  if (page !== null && !Object.hasOwn(FRESHNESS_PAGE_SCOPES, page)) {
    return { ok: false, status: 400, error: "unknown_page" };
  }

  const requested = url.searchParams
    .getAll("scope")
    .flatMap((value) => value.split(","))
    .map((value) => value.trim())
    .filter((value) => value.length > 0);

  if (requested.length > MAX_SCOPE_VALUES) return { ok: false, status: 400, error: "too_many_scopes" };
  for (const scope of requested) {
    if (!freshnessScopeAllowed(scope)) return { ok: false, status: 400, error: "invalid_scope" };
  }

  if (requested.length > 0) return { ok: true, scope: { page, scopes: new Set(requested) } };
  if (page !== null) return { ok: true, scope: { page, scopes: new Set(FRESHNESS_PAGE_SCOPES[page]!) } };
  return { ok: true, scope: { page: null, scopes: null } };
}

/** Only journal scopes the aggregator itself publishes are addressable from a
 *  browser. Anything else is rejected rather than silently ignored, so a
 *  caller-supplied scope can never be used to widen a view. */
export function freshnessScopeAllowed(scope: string): boolean {
  if (typeof scope !== "string" || scope.length === 0 || scope.length > 256) return false;
  return PAGE_SCOPE_NAMES.has(scope) || ITEM_SCOPE.test(scope);
}

export function parseAfter(value: string | null | undefined): number | null {
  if (value === null || value === undefined || value === "") return 0;
  if (!/^\d+$/.test(value)) return null;
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < 0 || parsed > MAX_AFTER) return null;
  return parsed;
}

function visible(notice: FreshnessNotice, scope: FreshnessDeliveryScope): boolean {
  if (scope.scopes === null) return true;
  return notice.scope.some((entry) => scope.scopes!.has(entry) && freshnessScopeAllowed(entry));
}

function sourceHealth(events: FreshnessNotice[], now: Date): Record<FreshnessSource, SourceHealth> {
  const health = Object.fromEntries(
    SOURCE_NAMES.map((source) => [source, { notices: 0, lastObservedAt: null, state: "silent" as const }]),
  ) as unknown as Record<FreshnessSource, SourceHealth>;
  for (const event of events) {
    const row = health[event.source];
    if (!row) continue;
    row.notices += 1;
    const at = Date.parse(event.observedAt);
    const age = now.getTime() - at;
    if (!Number.isFinite(age) || age < 0) continue;
    if (age <= LIVE_WINDOW_MS) row.state = "live";
    else if (age <= STALE_WINDOW_MS && row.state !== "live") row.state = "stale";
    if (row.lastObservedAt === null || event.observedAt > row.lastObservedAt) row.lastObservedAt = event.observedAt;
  }
  return health;
}

export async function readEnvelope(
  reader: FreshnessJournalReader,
  scope: FreshnessDeliveryScope,
  after: number,
  now: Date = new Date(),
): Promise<FreshnessEnvelope> {
  const page = await reader.listAfter(after);
  return {
    schemaVersion: 1,
    cursor: page.cursor,
    resetRequired: page.resetRequired,
    generatedAt: now.toISOString(),
    scopes: scope.scopes === null ? [...PAGE_SCOPE_NAMES].sort() : [...scope.scopes].sort(),
    events: page.events.filter((event) => visible(event, scope)),
    sources: sourceHealth(page.events, now),
  };
}

export function freshnessEventsJsonResponse(envelope: FreshnessEnvelope): Response {
  return new Response(JSON.stringify(envelope), {
    status: 200,
    headers: {
      "content-type": "application/json; charset=utf-8",
      "cache-control": "no-store",
      vary: "origin",
    },
  });
}

function sseFrame(fields: { id?: number; event?: string; data?: string; comment?: string }): string {
  const lines: string[] = [];
  if (fields.comment !== undefined) for (const line of fields.comment.split("\n")) lines.push(`: ${line}`);
  if (fields.id !== undefined) lines.push(`id: ${fields.id}`);
  if (fields.event !== undefined) lines.push(`event: ${fields.event}`);
  if (fields.data !== undefined) for (const line of fields.data.split("\n")) lines.push(`data: ${line}`);
  return `${lines.join("\n")}\n\n`;
}

export interface StreamOptions {
  reader: FreshnessJournalReader;
  scope: FreshnessDeliveryScope;
  /** Cursor to resume from (Last-Event-ID or `?after=`). */
  after: number;
  heartbeatMs?: number;
  pollMs?: number;
  now?: () => Date;
  /** Ends the stream (tests); production streams until the client leaves. */
  signal?: AbortSignal;
}

/**
 * The replayable SSE stream.
 *
 * Every notice is one `freshness` event whose SSE `id` is the journal
 * sequence, so an EventSource reconnect carries `Last-Event-ID` and the
 * retained range replays exactly once. A cursor older than the bounded
 * journal produces one `reset` event: the client refetches its whole view and
 * jumps to the current cursor instead of assuming it saw every change. The
 * stream never claims the data is fresh merely because it is connected, and
 * idle connections get a comment heartbeat so intermediaries keep them open.
 */
export function createFreshnessStream(options: StreamOptions): Response {
  const heartbeatMs = options.heartbeatMs ?? DEFAULT_HEARTBEAT_MS;
  const pollMs = options.pollMs ?? DEFAULT_POLL_MS;
  const now = options.now ?? (() => new Date());
  let closed = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let heartbeat: ReturnType<typeof setTimeout> | undefined;
  let controllerRef: ReadableStreamDefaultController<Uint8Array> | undefined;

  const send = (frame: string): void => {
    if (closed || controllerRef === undefined) return;
    try {
      controllerRef.enqueue(new TextEncoder().encode(frame));
    } catch {
      closed = true;
    }
  };
  const stop = (): void => {
    if (closed) return;
    closed = true;
    if (heartbeat) clearTimeout(heartbeat);
    if (timer) clearTimeout(timer);
    try {
      controllerRef?.close();
    } catch {
      // already closed by the client
    }
  };

  const scheduleHeartbeat = (): void => {
    heartbeat = setTimeout(() => {
      if (closed) return;
      send(sseFrame({ comment: `heartbeat ${now().toISOString()}` }));
      scheduleHeartbeat();
    }, heartbeatMs);
    // Do not hold the process open for a heartbeat alone.
    (heartbeat as unknown as { unref?: () => void }).unref?.();
  };

  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      controllerRef = controller;
      let cursor = options.after;
      let sentReset = false;

      options.signal?.addEventListener("abort", stop, { once: true });
      // One comment frame so headers flush and the client knows the stream is
      // live. It carries no data and is not a freshness claim.
      send(sseFrame({ comment: "freshness stream" }));

      const tick = async (): Promise<void> => {
        if (closed) return;
        try {
          const page = await options.reader.listAfter(cursor);
          if (page.resetRequired) {
            // Tell the client to refetch its whole view once, then advance to
            // the current cursor so the next tick resumes normally.
            if (!sentReset) {
              sentReset = true;
              send(
                sseFrame({
                  event: "reset",
                  data: JSON.stringify({ schemaVersion: 1, cursor: page.cursor, resetRequired: true }),
                }),
              );
            }
            cursor = page.cursor;
          } else {
            sentReset = false;
            let patched = false;
            for (const event of page.events) {
              if (event.sequence <= cursor) continue;
              if (visible(event, options.scope)) {
                send(sseFrame({ id: event.sequence, event: "freshness", data: JSON.stringify(event) }));
                // Coalesce: one render per tick, however many notices matched,
                // and reread canonical data rather than trusting the notice.
                patched = true;
              }
              // Invisible notices advance the cursor too: a reconnect must not
              // replay notices this connection was never allowed to see.
              cursor = Math.max(cursor, event.sequence);
            }
            if (patched && options.scope.page !== null) {
              const patch = freshnessPatchFrame(options.scope.page);
              if (patch !== null) send(patch);
            }
          }
        } catch {
          // A read failure is not a disconnect: the next tick retries and the
          // client keeps whatever it had, exactly like a failed canonical read.
        }
        if (closed) return;
        timer = setTimeout(() => void tick(), pollMs);
        (timer as unknown as { unref?: () => void }).unref?.();
      };

      scheduleHeartbeat();
      void tick();
    },
    cancel() {
      stop();
    },
  });

  return new Response(stream, {
    status: 200,
    headers: {
      "content-type": "text/event-stream; charset=utf-8",
      "cache-control": "no-cache, no-store",
      connection: "keep-alive",
      "x-accel-buffering": "no",
      vary: "origin",
    },
  });
}

export interface DeliveryOptions {
  reader: FreshnessJournalReader;
  /** Defaults to rejecting any cross-origin caller. The server passes its own
   *  browser authorization (the same policy the pages use). */
  authorize?: (request: Request, url: URL) => boolean | Promise<boolean>;
  now?: () => Date;
  heartbeatMs?: number;
  pollMs?: number;
  signal?: AbortSignal;
}

export function sameOrigin(request: Request, url: URL): boolean {
  const origin = request.headers.get("origin");
  if (origin === null) return true; // same-origin navigations and EventSource
  try {
    const parsed = new URL(origin);
    return parsed.protocol === url.protocol && parsed.host === url.host;
  } catch {
    return false;
  }
}

function errorResponse(status: number, error: string): Response {
  return new Response(JSON.stringify({ error }), {
    status,
    headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" },
  });
}

/**
 * Mount point for both surfaces. Returns null when the URL is not one of
 * them, so the aggregator's route table can chain to it unconditionally.
 */
export async function freshnessDeliveryResponse(
  request: Request,
  url: URL,
  options: DeliveryOptions,
): Promise<Response | null> {
  const isStream = url.pathname === "/freshness/stream";
  const isJson = url.pathname === "/api/freshness-events.json";
  if (!isStream && !isJson) return null;
  if (request.method !== "GET") return errorResponse(405, "method_not_allowed");

  const authorize = options.authorize ?? sameOrigin;
  if (!(await authorize(request, url))) return errorResponse(403, "forbidden");

  const parsed = deliveryScopeFromUrl(url);
  if (!parsed.ok) return errorResponse(parsed.status, parsed.error);

  // Reconnect cursor: the SSE standard header wins, the query parameter is
  // the JSON twin's cursor. Neither is a token and neither grants access.
  const rawHeader = isStream ? request.headers.get("last-event-id") : null;
  const headerCursor = rawHeader === null ? null : parseAfter(rawHeader);
  if (rawHeader !== null && headerCursor === null) return errorResponse(400, "invalid_after");
  const queryCursor = parseAfter(url.searchParams.get("after"));
  if (queryCursor === null) return errorResponse(400, "invalid_after");
  const after = headerCursor ?? queryCursor;

  if (isJson) {
    const envelope = await readEnvelope(options.reader, parsed.scope, after, options.now?.() ?? new Date());
    return freshnessEventsJsonResponse(envelope);
  }

  return createFreshnessStream({
    reader: options.reader,
    scope: parsed.scope,
    after,
    heartbeatMs: options.heartbeatMs,
    pollMs: options.pollMs,
    now: options.now,
    signal: options.signal,
  });
}
