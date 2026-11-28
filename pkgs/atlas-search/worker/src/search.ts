/**
 * The one search execution path (#6788): key the (normalized
 * query, k) pair under the pinned disclosure context (#12622)
 * and run it through the per-query Durable Object.
 *
 * Every surface that searches — the REST route and the MCP
 * endpoint (#8625) — calls this, so the ceiling/context pinning
 * and the cache keying can never drift between surfaces.
 */
import { queryKey } from "./key";
import { CONTEXT } from "./space";

export const DEFAULT_K = 10;
export const MAX_K = 50;

export function clampK(raw: unknown): number {
  const n = typeof raw === "number" ? Math.floor(raw) : DEFAULT_K;
  return Math.min(MAX_K, Math.max(1, Number.isFinite(n) ? n : DEFAULT_K));
}

/** The slice of the Worker environment the search path needs. */
export interface SearchEnv {
  QUERY_CACHE: DurableObjectNamespace;
}

export interface SearchOutcome {
  status: number;
  payload: Record<string, unknown>;
}

/**
 * Run one search. `query` must already be normalized and
 * non-empty (the callers' validation owns that); `k` must
 * already be clamped. On success the payload is the REST
 * envelope `{ cache, key, query, result, meta }`; on an
 * upstream failure it is the Durable Object's error payload,
 * with its status.
 */
export async function executeSearch(env: SearchEnv, query: string, k: number): Promise<SearchOutcome> {
  const key = await queryKey({ q: query, k, context: CONTEXT });
  const stub = env.QUERY_CACHE.get(env.QUERY_CACHE.idFromName(key));
  const resp = await stub.fetch("https://do/query", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ query, k }),
  });
  const payload = (await resp.json()) as Record<string, unknown>;
  if (!resp.ok) return { status: resp.status, payload };
  const { outcome, ...rest } = payload as { outcome: string };
  return { status: 200, payload: { cache: outcome, key, query, ...rest } };
}
