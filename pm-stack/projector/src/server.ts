// HTTP server: webhook intake + read API + link write API + reconcile trigger.
// Read-only by default; writes to the link registry and reconcile need the
// bearer token (PM_PROJECTOR_API_TOKEN). The webhook path is HMAC-gated.
import type { Client } from "@libsql/client";
import { applyEvent, type ProjectorStore } from "./projector";
import { validSignature, readBoundedBody, jsonResponse } from "./webhook";
import * as store from "./store";
import { reconcileRepos } from "./reconcile";

export interface ServerEnv extends Record<string, string | undefined> {
  GITHUB_WEBHOOK_SECRET?: string;
  PM_PROJECTOR_API_TOKEN?: string;
  PM_REPOS?: string; // comma-separated allow-list "owner/repo,..."
  DATABASE_URL?: string;
  TURSO_AUTH_TOKEN?: string;
  GITHUB_APP_ID?: string;
  GITHUB_APP_PRIVATE_KEY?: string;
  GITHUB_TOKEN?: string;
  /** Loopback-only pm-aggregator freshness publisher; absent in isolated tests. */
  PM_AGG_FRESHNESS_URL?: string;
}

interface FreshnessNoticeInput {
  eventId: string;
  source: "github";
  scope: string[];
  occurredAt: string | null;
  reason: "source-event";
}

function githubOccurredAt(payload: Record<string, any>): string | null {
  const candidates = [
    payload?.issue?.updated_at,
    payload?.pull_request?.updated_at,
    payload?.comment?.updated_at,
    payload?.review?.submitted_at,
    payload?.created_at,
  ];
  for (const value of candidates) {
    if (typeof value === "string" && Number.isFinite(Date.parse(value))) return new Date(value).toISOString();
  }
  return null;
}

async function publishFreshness(env: ServerEnv, input: FreshnessNoticeInput): Promise<void> {
  const url = env.PM_AGG_FRESHNESS_URL;
  // Tests and standalone projector deployments can omit the local sink.
  // The laptop unit configures it explicitly; no external route is enabled.
  if (!url) return;
  const response = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
    signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok) throw new Error(`freshness publisher returned ${response.status}`);
}

function libsqlStore(db: Client): ProjectorStore {
  return {
    upsertItem: (i) => store.upsertItem(db, i),
    upsertComment: (c) => store.upsertComment(db, c),
    markCommentDeleted: (c) => store.markCommentDeleted(db, c),
    deleteItem: (ref, sourceUpdatedAt) => store.deleteItem(db, ref, sourceUpdatedAt),
    itemDeletion: (ref) => store.getItemDeletion(db, ref),
    recordPush: (p) => store.recordPush(db, p),
    appendHotEvents: (events) => store.appendHotEvents(db, events),
  };
}

function repos(env: ServerEnv): string[] {
  return (env.PM_REPOS ?? "")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

export const DEFAULT_ITEMS_LIMIT = 200;
export const MAX_ITEMS_LIMIT = 1000;
export const DEFAULT_COMMENTS_LIMIT = 500;
export const MAX_COMMENTS_LIMIT = 1000;
export const DEFAULT_LINKS_LIMIT = 500;
export const MAX_LINKS_LIMIT = 1000;

/** Cap on the ref list accepted by the `refs` query parameter (#10595).
 *  Each ref becomes one SQL placeholder, so an unbounded list is both a row
 *  amplification and a host-variable-size hazard. The aggregator chunks its
 *  reads at 200 refs (PROJECTOR_REFS_PER_REQUEST), well under this. */
export const MAX_REFS = 500;

/** Cap on authenticated JSON write bodies (#10635). The webhook intake
 *  already reads through `readBoundedBody` at 10MB; `POST /links` and
 *  `POST /hot/checkpoints` used bare `req.json()`, letting one caller
 *  buffer an unbounded body before validation. Both now share this cap
 *  and answer 413 without reaching the store. */
export const MAX_WRITE_BODY_BYTES = 10_000_000;

/** Stored/compared string-field bounds (#10635). The body cap alone
 *  still admits a multi-MB `note` as a persistent row, so the fields
 *  that reach the store (or key a lookup) are bounded before any
 *  store call; violations are 400s. */
export const MAX_LINK_SUBJECT_CHARS = 1024;
export const MAX_LINK_NOTE_CHARS = 20_000;
export const MAX_LINK_RELATION_CHARS = 256;
export const MAX_LINK_CREATED_BY_CHARS = 256;
export const MAX_CONSUMER_CHARS = 256;

/** Read a JSON write body through the shared bounded-body discipline:
 *  413 on overflow, then JSON.parse with the caller's 400 on failure. */
async function readBoundedJson(
  req: Request,
): Promise<{ ok: true; value: any } | { ok: false; response: Response }> {
  let raw: Uint8Array;
  try {
    raw = await readBoundedBody(req.body, MAX_WRITE_BODY_BYTES);
  } catch {
    return { ok: false, response: jsonResponse(413, { error: "body_too_large" }) };
  }
  try {
    return { ok: true, value: JSON.parse(new TextDecoder().decode(raw)) };
  } catch {
    return { ok: false, response: jsonResponse(400, { error: "invalid_json" }) };
  }
}

/** Split a comma-separated `refs` parameter into a de-duplicated ref list. */
export function parseRefs(raw: string | null): string[] {
  if (!raw) return [];
  return [
    ...new Set(
      raw
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean),
    ),
  ];
}

/**
 * Parse the `limit` query parameter (#1547). Only a positive integer is
 * honoured, clamped to the endpoint maximum. An absent, non-numeric,
 * fractional, zero, or negative value falls back to the endpoint default:
 * an unvalidated value must never reach the store, where SQLite binds a
 * negative LIMIT as "no limit" and a fractional one raises an error that
 * escapes the handler as a 500.
 */
export function parseLimit(raw: string | null, defaultLimit: number, maxLimit: number): number {
  if (raw === null) return defaultLimit;
  // Only a decimal-digit spelling is honoured (#2971): Number() coercion
  // re-admits every spelling it accepts ('1e2', '0x10', '5.0', '+5', ' 5'),
  // which is the #1547 residual this gate exists to close.
  if (!/^[0-9]+$/.test(raw)) return defaultLimit;
  const n = Number(raw);
  if (n < 1) return defaultLimit;
  return Math.min(n, maxLimit);
}

/** Parse a hot-list cursor parameter: decimal digits only, else null. */
export function parseCursor(raw: string | null): number | null {
  if (raw === null || !/^[0-9]+$/.test(raw)) return null;
  const n = Number(raw);
  return Number.isSafeInteger(n) ? n : null;
}

/** A link ref is storable only if it is owner/repo#N with a safe-integer
 *  N - the same rule the aggregator's parseRef enforces (#4952, #6211).
 *  The shape regex alone admits numbers parseRef (and refUrl) throw on
 *  (e.g. a 20-digit issue number), and one such stored row 500s the
 *  aggregator's /search and project pages at render, so the write gate
 *  checks the number itself, not just the shape. */
export function isStorableLinkRef(ref: unknown): ref is string {
  if (typeof ref !== "string") return false;
  const m = /^[\w.-]+\/[\w.-]+#(\d+)$/.exec(ref);
  if (!m) return false;
  return Number.isSafeInteger(Number(m[1]));
}

function bearerAuthorized(req: Request, env: ServerEnv): boolean {
  const token = env.PM_PROJECTOR_API_TOKEN;
  if (!token) return false; // fail closed when no token configured
  const h = req.headers.get("authorization");
  return h === `Bearer ${token}`;
}

/**
 * Optional dedicated DB clients (#9850). The projector's /health stalls
 * came from sharing one libSQL client between the request handlers and
 * the reconcile write flood: the client's requests serialize, so a
 * health query queued behind thousands of reconcile upserts for
 * minutes at a time while the process stayed alive. Production wiring
 * (index.ts) passes a dedicated client for /health and one for the
 * reconcile trigger, so neither waits on the other's traffic; tests
 * and embedders that pass only `db` keep the previous behaviour.
 */
export interface ServerDeps {
  healthDb?: Client;
  reconcileDb?: Client;
  /** Background-refreshed /health snapshot source (#9850 round 2).
   *  Wired by serve(); handleRequest falls back to the bounded live
   *  probe when no snapshot source is provided. */
  healthSnapshot?: () => HealthSnapshot | null;
  /** Single-flight gate for POST /reconcile (#9850 round 2), wired by
   *  serve(): beginReconcile returns false while a pass is in flight. */
  beginReconcile?: () => boolean;
  endReconcile?: () => void;
}

/** A point-in-time row-count snapshot served by /health (#9850 round 2). */
export interface HealthSnapshot {
  items: number;
  comments: number;
  links: number;
  /** Epoch ms when the snapshot was taken. */
  at: number;
}

/** A health snapshot older than this is itself evidence the process is
 *  wedged (its refresher has not completed in that long) → 503. */
export const HEALTH_SNAPSHOT_STALE_MS = 120_000;
/** How often serve() refreshes the health snapshot in the background. */
export const HEALTH_REFRESH_MS = 15_000;

const HEALTH_COUNT_SQL =
  "SELECT (SELECT COUNT(*) FROM pmp_items) AS items, (SELECT COUNT(*) FROM pmp_comments) AS comments, (SELECT COUNT(*) FROM pmp_links) AS links";

async function readHealthCounts(client: Client): Promise<{ items: number; comments: number; links: number }> {
  const r = await withTimeout(client.execute(HEALTH_COUNT_SQL), HEALTH_DB_TIMEOUT_MS, "health_db_timeout");
  return r.rows[0] as unknown as { items: number; comments: number; links: number };
}

/** /health's DB probe is bounded: a health answer later than this is a down answer. */
export const HEALTH_DB_TIMEOUT_MS = 5_000;

async function withTimeout<T>(work: Promise<T>, ms: number, label: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      work,
      new Promise<never>((_resolve, reject) => {
        timer = setTimeout(() => reject(new Error(label)), ms);
      }),
    ]);
  } finally {
    if (timer) clearTimeout(timer);
  }
}

/**
 * The service's error boundary (#10540): every route answers JSON. A
 * store, git or reconcile failure a route does not handle itself would
 * otherwise escape as Bun's default non-JSON 500, which clients and
 * monitors cannot read. Routes that already answer a specific status
 * keep it; only an escaped exception becomes this 500.
 */
export async function handleRequest(
  req: Request,
  db: Client,
  env: ServerEnv,
  deps: ServerDeps = {},
): Promise<Response> {
  try {
    return await routeRequest(req, db, env, deps);
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    const path = (() => {
      try {
        return new URL(req.url).pathname;
      } catch {
        return "?";
      }
    })();
    console.error(`projector: ${req.method} ${path} failed: ${message}`);
    return jsonResponse(500, { error: "internal_error", message });
  }
}

async function routeRequest(
  req: Request,
  db: Client,
  env: ServerEnv,
  deps: ServerDeps = {},
): Promise<Response> {
  const url = new URL(req.url);
  const path = url.pathname;
  const method = req.method;

  // -- health ---------------------------------------------------------------
  if (path === "/health" || path === "/api/health") {
    // #9850 round 2: when serve() provides a snapshot, /health does NO
    // database work on the request path — it serves counts a background
    // refresher keeps current, so reconcile write load can neither
    // stall the answer past the sentinel's probe timeout nor turn a
    // slow-but-alive process into a "down" alert. The staleness bound
    // keeps the signal honest: a genuinely wedged event loop cannot
    // refresh, its snapshot ages out, and the answer becomes 503.
    const snapshot = deps.healthSnapshot?.() ?? null;
    if (snapshot) {
      const snapshotAgeMs = Date.now() - snapshot.at;
      if (snapshotAgeMs <= HEALTH_SNAPSHOT_STALE_MS) {
        return jsonResponse(200, {
          ok: true,
          items: snapshot.items,
          comments: snapshot.comments,
          links: snapshot.links,
          snapshotAgeMs,
        });
      }
      return jsonResponse(503, { ok: false, error: "health_snapshot_stale", snapshotAgeMs });
    }
    try {
      const row = await readHealthCounts(deps.healthDb ?? db);
      return jsonResponse(200, { ok: true, ...row });
    } catch (e) {
      return jsonResponse(503, { ok: false, error: String((e as Error).message) });
    }
  }

  // -- webhook intake -------------------------------------------------------
  if (path === "/github/webhook" && method === "POST") {
    const signature = req.headers.get("x-hub-signature-256");
    const event = req.headers.get("x-github-event") ?? "unknown";
    const deliveryId = req.headers.get("x-github-delivery") ?? "";
    if (!deliveryId) {
      return jsonResponse(400, { error: "missing_delivery_id" });
    }
    let body: Uint8Array;
    try {
      body = await readBoundedBody(req.body, 10_000_000);
    } catch {
      return jsonResponse(413, { error: "body_too_large" });
    }
    if (!(await validSignature(body, signature, env.GITHUB_WEBHOOK_SECRET))) {
      return jsonResponse(401, { error: "invalid_signature" });
    }
    const already = await store.deliveryApplied(db, deliveryId);
    if (already) {
      return jsonResponse(200, { status: "duplicate" });
    }
    let payload: Record<string, any>;
    try {
      payload = JSON.parse(new TextDecoder().decode(body));
    } catch {
      await store.recordDelivery(db, { deliveryId, event, outcome: "error", detail: "invalid_json" });
      return jsonResponse(400, { error: "invalid_json" });
    }
    try {
      const result = await applyEvent(libsqlStore(db), event, payload, repos(env), deliveryId);
      if (result.wrote && result.refs.length > 0) {
        // Append before recording this delivery as applied. If the
        // aggregator is unavailable, GitHub's retry re-applies the
        // projection and retries the same stable delivery id; an
        // already-applied row must never hide a missing invalidation.
        await publishFreshness(env, {
          eventId: `github:${deliveryId}`,
          source: "github",
          scope: [...new Set(result.refs)].slice(0, 100),
          occurredAt: githubOccurredAt(payload),
          reason: "source-event",
        });
      }
      await store.recordDelivery(db, {
        deliveryId,
        event,
        action: payload?.action ?? null,
        ref: result.refs[0] ?? null,
        outcome: result.wrote ? "applied" : "ignored",
        detail: result.wrote ? undefined : "no_writes",
      });
      return jsonResponse(200, { status: result.wrote ? "applied" : "ignored", refs: result.refs });
    } catch (e) {
      await store.recordDelivery(db, { deliveryId, event, outcome: "error", detail: String((e as Error).message) });
      return jsonResponse(500, { error: "apply_failed" });
    }
  }

  // -- read: items ----------------------------------------------------------
  if (path === "/items" && method === "GET") {
    const refsParam = url.searchParams.get("refs");
    if (refsParam) {
      const refs = parseRefs(refsParam);
      if (refs.length > MAX_REFS) {
        return jsonResponse(400, { error: "too_many_refs", max: MAX_REFS });
      }
      const rows = await store.getItems(db, refs, {
        limit: parseLimit(url.searchParams.get("limit"), DEFAULT_ITEMS_LIMIT, MAX_ITEMS_LIMIT),
      });
      return jsonResponse(200, { items: rows });
    }
    const rows = await store.listItems(db, {
      repo: url.searchParams.get("repo") ?? undefined,
      state: url.searchParams.get("state") ?? undefined,
      kind: url.searchParams.get("kind") ?? undefined,
      limit: parseLimit(url.searchParams.get("limit"), DEFAULT_ITEMS_LIMIT, MAX_ITEMS_LIMIT),
    });
    return jsonResponse(200, { items: rows });
  }

  // -- read: comments -------------------------------------------------------
  if (path === "/comments" && method === "GET") {
    const refsParam = url.searchParams.get("refs");
    const subject = url.searchParams.get("subject");
    const limit = parseLimit(url.searchParams.get("limit"), DEFAULT_COMMENTS_LIMIT, MAX_COMMENTS_LIMIT);
    if (subject) {
      const rows = await store.getCommentsForSubject(db, subject, { limit });
      return jsonResponse(200, { comments: rows });
    }
    const refs = parseRefs(refsParam);
    if (refs.length > MAX_REFS) {
      return jsonResponse(400, { error: "too_many_refs", max: MAX_REFS });
    }
    const rows = await store.getComments(db, refs, { limit });
    return jsonResponse(200, { comments: rows });
  }

  // -- read/write: links ----------------------------------------------------
  if (path === "/links") {
    if (method === "GET") {
      const subject = url.searchParams.get("subject");
      const ref = url.searchParams.get("ref");
      const limit = parseLimit(url.searchParams.get("limit"), DEFAULT_LINKS_LIMIT, MAX_LINKS_LIMIT);
      const offset = parseCursor(url.searchParams.get("offset")) ?? 0;
      const rows = await store.getLinks(db, {
        subject: subject ?? undefined,
        ref: ref ?? undefined,
        limit,
        offset,
      });
      return jsonResponse(200, {
        links: rows,
        next_offset: rows.length === limit ? offset + limit : null,
      });
    }
    if (method === "POST" && bearerAuthorized(req, env)) {
      const parsed = await readBoundedJson(req);
      if (!parsed.ok) return parsed.response;
      const body: any = parsed.value;
      const { subject, ref, relation = "relates", note, createdBy } = body ?? {};
      if (typeof subject !== "string" || subject.length === 0 || typeof ref !== "string" || !isStorableLinkRef(ref)) {
        return jsonResponse(400, { error: "subject (non-empty string) and ref (owner/repo#N) required" });
      }
      if (subject.length > MAX_LINK_SUBJECT_CHARS) {
        return jsonResponse(400, { error: "subject too long", max: MAX_LINK_SUBJECT_CHARS });
      }
      // Validate the whole body before any store call (#2974): relation is
      // never String()-coerced (an array stored as 'a,b', an object as
      // '[object Object]'), and note/createdBy are bound by the store, so a
      // non-string there threw the SQLite bind error out of the handler.
      if (typeof relation !== "string") {
        return jsonResponse(400, { error: "relation (string) required" });
      }
      if (relation.length > MAX_LINK_RELATION_CHARS) {
        return jsonResponse(400, { error: "relation too long", max: MAX_LINK_RELATION_CHARS });
      }
      if (note !== undefined && note !== null && typeof note !== "string") {
        return jsonResponse(400, { error: "note (string or null) required" });
      }
      if (typeof note === "string" && note.length > MAX_LINK_NOTE_CHARS) {
        return jsonResponse(400, { error: "note too long", max: MAX_LINK_NOTE_CHARS });
      }
      if (createdBy !== undefined && createdBy !== null && typeof createdBy !== "string") {
        return jsonResponse(400, { error: "createdBy (string or null) required" });
      }
      if (typeof createdBy === "string" && createdBy.length > MAX_LINK_CREATED_BY_CHARS) {
        return jsonResponse(400, { error: "createdBy too long", max: MAX_LINK_CREATED_BY_CHARS });
      }
      try {
        await store.upsertLink(db, { subject, ref, relation, note, createdBy });
      } catch (err) {
        return jsonResponse(500, { error: `link_store_failed: ${err instanceof Error ? err.message : String(err)}` });
      }
      return jsonResponse(201, { ok: true, subject, ref, relation });
    }
    if (method === "DELETE" && bearerAuthorized(req, env)) {
      const subject = url.searchParams.get("subject");
      const ref = url.searchParams.get("ref");
      const relation = url.searchParams.get("relation") ?? "relates";
      if (!subject || !ref) return jsonResponse(400, { error: "subject and ref required" });
      await store.deleteLink(db, subject, ref, relation);
      return jsonResponse(200, { ok: true });
    }
    return jsonResponse(401, { error: "unauthorized" });
  }

  // -- read: joined subject ---------------------------------------------------
  if (path.startsWith("/subjects/") && method === "GET") {
    // A malformed percent-encoding is a 400, never a 500: an unguarded
    // decodeURIComponent throws URIError out of the handler (#2693,
    // design #2747 Phase 2; same guard as #2720 in cf-container-hello).
    // Only the decode is guarded \u2014 a well-formed subject still
    // resolves (or 404s) exactly as before.
    let subject: string;
    try {
      subject = decodeURIComponent(path.slice("/subjects/".length));
    } catch {
      return jsonResponse(400, { error: "malformed percent-encoding in subject" });
    }
    const view = await store.getSubject(db, subject);
    if (!view) return jsonResponse(404, { error: "subject_not_found" });
    return jsonResponse(200, view);
  }

  // -- hot list (#5204): append-only change ledger + consumer checkpoints --
  if (path === "/hot" && method === "GET") {
    const after = parseCursor(url.searchParams.get("after") ?? url.searchParams.get("cursor")) ?? 0;
    const limit = parseLimit(url.searchParams.get("limit"), DEFAULT_ITEMS_LIMIT, MAX_ITEMS_LIMIT);
    // A sweep pins the high-water the server issues at its start; pages
    // requested with that high-water never see events past it, so a sweep
    // is a fixed window and its checkpoint can ack up to it exactly.
    const pinned = parseCursor(url.searchParams.get("high_water"));
    // The issued high-water is always server-controlled: a caller-supplied
    // pin is clamped to the ledger's actual high-water, never echoed past
    // it, so the value a sweep acks against is one the server truly issued.
    const serverHighWater = await store.hotHighWater(db);
    const highWater = pinned === null ? serverHighWater : Math.min(pinned, serverHighWater);
    const events = await store.listHotEvents(db, { after, limit, highWater });
    return jsonResponse(200, {
      events,
      high_water: highWater,
      next_cursor: events.length ? events[events.length - 1].seq : after,
    });
  }

  if (path === "/hot/checkpoints" && method === "GET") {
    const consumer = url.searchParams.get("consumer");
    if (!consumer) return jsonResponse(400, { error: "consumer required" });
    const cursor = await store.getConsumerCheckpoint(db, consumer);
    return jsonResponse(200, { consumer, cursor });
  }

  if (path === "/hot/checkpoints" && method === "POST") {
    if (!bearerAuthorized(req, env)) return jsonResponse(401, { error: "unauthorized" });
    const parsed = await readBoundedJson(req);
    if (!parsed.ok) return parsed.response;
    const body: any = parsed.value;
    const { consumer, cursor, highWater } = body ?? {};
    if (typeof consumer !== "string" || consumer.length === 0) {
      return jsonResponse(400, { error: "consumer (non-empty string) required" });
    }
    if (consumer.length > MAX_CONSUMER_CHARS) {
      return jsonResponse(400, { error: "consumer too long", max: MAX_CONSUMER_CHARS });
    }
    if (!Number.isInteger(cursor) || !Number.isInteger(highWater)) {
      return jsonResponse(400, { error: "cursor and highWater (integers) required" });
    }
    try {
      const stored = await store.ackConsumerCheckpoint(db, consumer, cursor, highWater);
      return jsonResponse(200, { consumer, cursor: stored });
    } catch (e) {
      return jsonResponse(400, { error: String((e as Error).message) });
    }
  }

  // -- reconcile trigger ------------------------------------------------------
  if (path === "/reconcile" && method === "POST") {
    if (!bearerAuthorized(req, env)) return jsonResponse(401, { error: "unauthorized" });
    // Single-flight (#9850 round 2): the reconcile timer fires every
    // ~15 minutes whether or not the previous pass has finished, and
    // overlapping passes multiply exactly the GitHub + write load that
    // stalls the box. A trigger that arrives mid-pass is a no-op.
    if (deps.beginReconcile && !deps.beginReconcile()) {
      return jsonResponse(200, { ok: true, skipped: "reconcile_in_progress" });
    }
    try {
      const result = await reconcileRepos(deps.reconcileDb ?? db, env);
      return jsonResponse(200, result);
    } finally {
      deps.endReconcile?.();
    }
  }

  return jsonResponse(404, { error: "not_found" });
}

export function serve(db: Client, env: ServerEnv, port: number, deps: ServerDeps = {}): Bun.Server {
  // #9850 round 2 wiring: a background health-snapshot refresher and
  // the reconcile single-flight gate, so the request path never waits
  // on projection work. Explicit deps win (tests, embedders).
  let snapshot: HealthSnapshot | null = null;
  const refreshHealth = async () => {
    try {
      snapshot = { ...(await readHealthCounts(deps.healthDb ?? db)), at: Date.now() };
    } catch {
      // Keep the last good snapshot; /health's staleness bound turns a
      // persistently failing refresher into a 503 on its own.
    }
  };
  void refreshHealth();
  const healthTimer = setInterval(() => void refreshHealth(), HEALTH_REFRESH_MS);
  (healthTimer as unknown as { unref?: () => void }).unref?.();

  let reconcileRunning = false;
  const wiredDeps: ServerDeps = {
    ...deps,
    healthSnapshot: deps.healthSnapshot ?? (() => snapshot),
    beginReconcile:
      deps.beginReconcile ??
      (() => {
        if (reconcileRunning) return false;
        reconcileRunning = true;
        return true;
      }),
    endReconcile:
      deps.endReconcile ??
      (() => {
        reconcileRunning = false;
      }),
  };
  return Bun.serve({
    port,
    hostname: env.HOSTNAME ?? "127.0.0.1",
    // A reconcile poll runs for minutes; Bun's 10s default idle timeout
    // closed the socket mid-poll and the caller saw an empty reply.
    idleTimeout: 255,
    async fetch(req, server) {
      // /reconcile is a long poll by design: under GitHub rate
      // limiting a full pass takes longer than Bun's 255s
      // idle-timeout ceiling, and the reconcile timer unit's curl
      // died at the cap with an empty reply (curl 52) even though
      // the poll itself was healthy (2026-11-01). Exempt just this
      // route from the idle timeout; every other route keeps 255s.
      if (new URL(req.url).pathname === "/reconcile") server.timeout(req, 0);
      return handleRequest(req, db, env, wiredDeps);
    },
  });
}
