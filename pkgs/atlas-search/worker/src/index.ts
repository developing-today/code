/**
 * atlas-search Worker (#6788): the cross-database search surface.
 *
 * Drew's spec: a Worker that pulls all databases for search; for
 * each query it stores a Durable Object, automatically inserts
 * the query as an embedding, and caches the result indefinitely.
 *
 * The Worker itself holds no database credentials: fan-out,
 * embedding, and merge live in the fleet search service (see
 * service.ts); this Worker's jobs are the inbound gate, query
 * normalization/keying, and the per-query Durable Object cache.
 *
 * Routes:
 *   GET  /health                    (open; liveness only)
 *   POST /v1/search  {query, k?, session?} (gated) -> miss or hit envelope
 *   GET  /v1/cache/inspect?query=&k= (gated) -> DO metadata readback
 *   POST /v1/cache/purge {query, k?} (gated) -> delete one cached query
 *   POST /mcp                       (gated) -> MCP endpoint (#8625; src/mcp.ts)
 *   POST /v1/admin/retained/audit {object, version} (gated) -> pin a version for audit (#9140)
 *   GET  /v1/admin/retained/compare?object=&from=&to= (gated) -> before/after report (#9140)
 */
import { readTextUpTo } from "./body";
import { QueryCache } from "./cache_do";
import { normalizeQuery, queryKey } from "./key";
import { handleMcp } from "./mcp";
import { clampK, DEFAULT_K, doJsonObject, executeSearch, malformedDoPayload } from "./search";
import { AuditError, compareAudited, pinForAudit } from "./audit_report";
import { RetainedObjectError } from "./retained_object";
import { recordSearch, validSession } from "./search_history";
import { CONTEXT } from "./space";

export { QueryCache };

export interface Env {
  QUERY_CACHE: DurableObjectNamespace;
  /** D1 tenant hot cache binding (#9134 §5, #13100); optional so a
   * deploy without the database provisioned still typechecks and
   * runs the pre-cache path. */
  HOT_CACHE?: D1Database;
  /** Hot-cache master flag; only the literal "true" enables it. */
  HOT_CACHE_ENABLED?: string;
  /** Retained objects (#9140): search history for callers that name a
   * session, kept only while its class is enabled there, and the admin
   * audit set the comparison report reads. */
  RETAINED?: D1Database;
  GATE_TOKEN: string;
  SERVICE_BASE: string;
  SERVICE_TOKEN: string;
  ACCESS_CLIENT_ID: string;
  ACCESS_CLIENT_SECRET: string;
}

function errorJson(status: number, code: string, message: string): Response {
  return Response.json({ error: { code, message } }, { status });
}

/** A positive whole version number, or undefined. */
function versionNumber(value: unknown): number | undefined {
  const n = typeof value === "string" && /^[0-9]{1,15}$/.test(value) ? Number(value) : value;
  return typeof n === "number" && Number.isSafeInteger(n) && n >= 1 ? n : undefined;
}

/** The admin audit routes (#9140 slice 5). */
async function retainedAdmin(req: Request, env: Env, url: URL): Promise<Response> {
  const db = env.RETAINED;
  if (!db) return errorJson(503, "retained_off", "no retained-object store is configured");
  try {
    if (req.method === "POST" && url.pathname === "/v1/admin/retained/audit") {
      const raw = await readTextUpTo(req);
      if (raw === null) return errorJson(413, "payload_too_large", "body is larger than the 64 KiB limit");
      let body: unknown;
      try {
        body = JSON.parse(raw);
      } catch {
        return errorJson(400, "bad_request", "body must be JSON");
      }
      const { object, version } = (typeof body === "object" && body !== null ? body : {}) as {
        object?: unknown;
        version?: unknown;
      };
      const v = versionNumber(version);
      if (typeof object !== "string" || object.length === 0 || v === undefined) {
        return errorJson(400, "bad_request", "object must be a non-empty string and version a positive integer");
      }
      return Response.json(await pinForAudit(db, object, v));
    }
    if (req.method === "GET" && url.pathname === "/v1/admin/retained/compare") {
      const object = url.searchParams.get("object");
      const from = versionNumber(url.searchParams.get("from"));
      const to = versionNumber(url.searchParams.get("to"));
      if (!object || from === undefined || to === undefined) {
        return errorJson(400, "bad_request", "object, from and to are required; from and to are positive integers");
      }
      return Response.json(await compareAudited(db, object, from, to));
    }
  } catch (error) {
    if (error instanceof AuditError) {
      return errorJson(error.code === "family_mismatch" ? 409 : 404, error.code, error.message);
    }
    if (error instanceof RetainedObjectError) return errorJson(409, error.code, error.message);
    throw error;
  }
  return errorJson(405, "method_not_allowed", "use POST for audit and GET for compare");
}

/** Constant-time bearer comparison (via digests, so length-safe). */
async function gateOk(env: Env, req: Request): Promise<boolean> {
  const auth = req.headers.get("Authorization") ?? "";
  const presented = auth.startsWith("Bearer ") ? auth.slice(7) : "";
  if (!presented || !env.GATE_TOKEN) return false;
  const enc = new TextEncoder();
  const [a, b] = await Promise.all([
    crypto.subtle.digest("SHA-256", enc.encode(presented)),
    crypto.subtle.digest("SHA-256", enc.encode(env.GATE_TOKEN)),
  ]);
  const av = new Uint8Array(a);
  const bv = new Uint8Array(b);
  let diff = 0;
  for (let i = 0; i < av.length; i++) diff |= av[i] ^ bv[i];
  return diff === 0;
}

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    const url = new URL(req.url);

    if (req.method === "GET" && url.pathname === "/health") {
      return Response.json({ status: "ok", service: "atlas-search-worker" });
    }

    if (req.method === "POST" && url.pathname === "/v1/search") {
      if (!(await gateOk(env, req))) {
        return errorJson(401, "unauthorized", "missing or invalid gate token");
      }
      // #11331: bounded read; the route only needs { query, k }.
      const raw = await readTextUpTo(req);
      if (raw === null) {
        return errorJson(413, "payload_too_large", "body is larger than the 64 KiB limit");
      }
      let body: { query?: unknown; k?: unknown; session?: unknown };
      try {
        body = JSON.parse(raw) as typeof body;
      } catch {
        return errorJson(400, "bad_request", "body must be JSON");
      }
      // `null` is valid JSON, so the parse guard above lets it through;
      // every property read below would throw out of the handler and
      // surface as a platform 500 instead of this route's envelope
      // (#6879). Reject non-object bodies before dereferencing.
      if (typeof body !== "object" || body === null) {
        return errorJson(400, "bad_request", "body must be a JSON object");
      }
      if (typeof body.query !== "string") {
        return errorJson(400, "bad_request", "query must be a string");
      }
      const query = normalizeQuery(body.query);
      if (!query) return errorJson(400, "bad_request", "query must not be empty");
      const session = body.session === undefined ? undefined : validSession(body.session) ? body.session : null;
      if (session === null) {
        return errorJson(400, "bad_request", "session must be 1 to 128 of A-Z, a-z, 0-9, '.', '_', ':' or '-'");
      }
      const k = clampK(body.k);
      const { status, payload } = await executeSearch(env, query, k);
      if (status === 200 && session !== undefined) {
        // Search history (#9140 slice 3) is best effort: it never changes
        // or fails the answer.
        try {
          await recordSearch(env.RETAINED, {
            session,
            query,
            k,
            key: String(payload.key),
            cache: payload.cache === "hit" ? "hit" : "miss",
            result: payload.result,
          });
        } catch (error) {
          console.warn(`search history not recorded: ${(error as Error).message}`);
        }
      }
      return Response.json(payload, { status });
    }

    if (req.method === "GET" && url.pathname === "/v1/cache/inspect") {
      if (!(await gateOk(env, req))) {
        return errorJson(401, "unauthorized", "missing or invalid gate token");
      }
      const q = url.searchParams.get("query");
      if (!q) return errorJson(400, "bad_request", "query parameter is required");
      const query = normalizeQuery(q);
      // Same post-normalization check as POST /v1/search (#6937): a
      // whitespace-only query is empty once normalized and must not be
      // keyed or reach a Durable Object.
      if (!query) return errorJson(400, "bad_request", "query must not be empty");
      const k = clampK(Number(url.searchParams.get("k") ?? DEFAULT_K));
      const key = await queryKey({ q: query, k, context: CONTEXT });
      const stub = env.QUERY_CACHE.get(env.QUERY_CACHE.idFromName(key));
      const resp = await stub.fetch("https://do/inspect");
      const meta = await doJsonObject(resp);
      if (meta === null) return Response.json(malformedDoPayload("inspect"), { status: 502 });
      if (!resp.ok) return Response.json(meta, { status: resp.status });
      return Response.json({ key, query, ...meta });
    }

    if (req.method === "POST" && url.pathname === "/v1/cache/purge") {
      if (!(await gateOk(env, req))) {
        return errorJson(401, "unauthorized", "missing or invalid gate token");
      }
      const raw = await readTextUpTo(req);
      if (raw === null) {
        return errorJson(413, "payload_too_large", "body is larger than the 64 KiB limit");
      }
      let body: { query?: unknown; k?: unknown };
      try {
        body = JSON.parse(raw) as typeof body;
      } catch {
        return errorJson(400, "bad_request", "body must be JSON");
      }
      if (typeof body !== "object" || body === null) {
        return errorJson(400, "bad_request", "body must be a JSON object");
      }
      if (typeof body.query !== "string") {
        return errorJson(400, "bad_request", "query must be a string");
      }
      const query = normalizeQuery(body.query);
      if (!query) return errorJson(400, "bad_request", "query must not be empty");
      const k = clampK(body.k);
      const key = await queryKey({ q: query, k, context: CONTEXT });
      const stub = env.QUERY_CACHE.get(env.QUERY_CACHE.idFromName(key));
      const resp = await stub.fetch("https://do/purge", { method: "POST" });
      const result = await doJsonObject(resp);
      if (result !== null && !resp.ok) return Response.json(result, { status: resp.status });
      if (result === null || typeof result.purged !== "boolean") {
        return Response.json(malformedDoPayload("purge"), { status: 502 });
      }
      return Response.json({ key, query, purged: result.purged });
    }

    if (url.pathname === "/v1/admin/retained/audit" || url.pathname === "/v1/admin/retained/compare") {
      if (!(await gateOk(env, req))) {
        return errorJson(401, "unauthorized", "missing or invalid gate token");
      }
      return retainedAdmin(req, env, url);
    }

    if (url.pathname === "/mcp") {
      // The MCP endpoint (#8625; src/mcp.ts) is POST-only and
      // sits behind the same gate as /v1/*. The gate runs here
      // so mcp.ts owns the JSON-RPC layer only.
      if (req.method !== "POST") {
        const resp = errorJson(405, "method_not_allowed", "the MCP endpoint accepts POST only");
        resp.headers.set("Allow", "POST");
        return resp;
      }
      if (!(await gateOk(env, req))) {
        return errorJson(401, "unauthorized", "missing or invalid gate token");
      }
      return handleMcp(req, env);
    }

    return errorJson(404, "not_found", "no route");
  },
};
