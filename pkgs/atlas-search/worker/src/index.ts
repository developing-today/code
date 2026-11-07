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
 *   POST /v1/search  {query, k?}    (gated) -> miss or hit envelope
 *   GET  /v1/cache/inspect?query=&k= (gated) -> DO metadata readback
 */
import { QueryCache } from "./cache_do";
import { normalizeQuery, queryKey } from "./key";

export { QueryCache };

export interface Env {
  QUERY_CACHE: DurableObjectNamespace;
  GATE_TOKEN: string;
  SERVICE_BASE: string;
  SERVICE_TOKEN: string;
  ACCESS_CLIENT_ID: string;
  ACCESS_CLIENT_SECRET: string;
}

const DEFAULT_K = 10;
const MAX_K = 50;

function errorJson(status: number, code: string, message: string): Response {
  return Response.json({ error: { code, message } }, { status });
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

function clampK(raw: unknown): number {
  const n = typeof raw === "number" ? Math.floor(raw) : DEFAULT_K;
  return Math.min(MAX_K, Math.max(1, Number.isFinite(n) ? n : DEFAULT_K));
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
      let body: { query?: unknown; k?: unknown };
      try {
        body = (await req.json()) as typeof body;
      } catch {
        return errorJson(400, "bad_request", "body must be JSON");
      }
      if (typeof body.query !== "string") {
        return errorJson(400, "bad_request", "query must be a string");
      }
      const query = normalizeQuery(body.query);
      if (!query) return errorJson(400, "bad_request", "query must not be empty");
      const k = clampK(body.k);
      const key = await queryKey({ q: query, k });
      const stub = env.QUERY_CACHE.get(env.QUERY_CACHE.idFromName(key));
      const resp = await stub.fetch("https://do/query", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ query, k }),
      });
      const payload = (await resp.json()) as Record<string, unknown>;
      if (!resp.ok) return Response.json(payload, { status: resp.status });
      const { outcome, ...rest } = payload as { outcome: string };
      return Response.json({ cache: outcome, key, query, ...rest });
    }

    if (req.method === "GET" && url.pathname === "/v1/cache/inspect") {
      if (!(await gateOk(env, req))) {
        return errorJson(401, "unauthorized", "missing or invalid gate token");
      }
      const q = url.searchParams.get("query");
      if (!q) return errorJson(400, "bad_request", "query parameter is required");
      const query = normalizeQuery(q);
      const k = clampK(Number(url.searchParams.get("k") ?? DEFAULT_K));
      const key = await queryKey({ q: query, k });
      const stub = env.QUERY_CACHE.get(env.QUERY_CACHE.idFromName(key));
      const resp = await stub.fetch("https://do/inspect");
      const meta = (await resp.json()) as Record<string, unknown>;
      return Response.json({ key, query, ...meta });
    }

    return errorJson(404, "not_found", "no route");
  },
};
