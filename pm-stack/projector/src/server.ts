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
}

function libsqlStore(db: Client): ProjectorStore {
  return {
    upsertItem: (i) => store.upsertItem(db, i),
    upsertComment: (c) => store.upsertComment(db, c),
    markCommentDeleted: (id, kind) => store.markCommentDeleted(db, id, kind),
    recordPush: (p) => store.recordPush(db, p),
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

function bearerAuthorized(req: Request, env: ServerEnv): boolean {
  const token = env.PM_PROJECTOR_API_TOKEN;
  if (!token) return false; // fail closed when no token configured
  const h = req.headers.get("authorization");
  return h === `Bearer ${token}`;
}

export async function handleRequest(req: Request, db: Client, env: ServerEnv): Promise<Response> {
  const url = new URL(req.url);
  const path = url.pathname;
  const method = req.method;

  // -- health ---------------------------------------------------------------
  if (path === "/health" || path === "/api/health") {
    try {
      const r = await db.execute(
        "SELECT (SELECT COUNT(*) FROM pmp_items) AS items, (SELECT COUNT(*) FROM pmp_comments) AS comments, (SELECT COUNT(*) FROM pmp_links) AS links",
      );
      const row = r.rows[0] as unknown as Record<string, number>;
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
      const refs = refsParam
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean);
      const rows = await store.getItems(db, refs);
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
    const refs = (refsParam ?? "")
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    const rows = await store.getComments(db, refs, { limit });
    return jsonResponse(200, { comments: rows });
  }

  // -- read/write: links ----------------------------------------------------
  if (path === "/links") {
    if (method === "GET") {
      const subject = url.searchParams.get("subject");
      const ref = url.searchParams.get("ref");
      const rows = await store.getLinks(db, { subject: subject ?? undefined, ref: ref ?? undefined });
      return jsonResponse(200, { links: rows });
    }
    if (method === "POST" && bearerAuthorized(req, env)) {
      let body: any;
      try {
        body = await req.json();
      } catch {
        return jsonResponse(400, { error: "invalid_json" });
      }
      const { subject, ref, relation = "relates", note, createdBy } = body ?? {};
      if (typeof subject !== "string" || typeof ref !== "string" || !/^[\w.-]+\/[\w.-]+#\d+$/.test(ref)) {
        return jsonResponse(400, { error: "subject (string) and ref (owner/repo#N) required" });
      }
      await store.upsertLink(db, { subject, ref, relation: String(relation), note, createdBy });
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

  // -- reconcile trigger ------------------------------------------------------
  if (path === "/reconcile" && method === "POST") {
    if (!bearerAuthorized(req, env)) return jsonResponse(401, { error: "unauthorized" });
    const result = await reconcileRepos(db, env);
    return jsonResponse(200, result);
  }

  return jsonResponse(404, { error: "not_found" });
}

export function serve(db: Client, env: ServerEnv, port: number): Bun.Server {
  return Bun.serve({
    port,
    hostname: env.HOSTNAME ?? "127.0.0.1",
    // A reconcile poll runs for minutes; Bun's 10s default idle timeout
    // closed the socket mid-poll and the caller saw an empty reply.
    idleTimeout: 255,
    async fetch(req) {
      return handleRequest(req, db, env);
    },
  });
}
