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
import { tryServeFromHotCache, type HotCacheReader, type SkipLog } from "./read_path";
import type { WatermarkSource } from "./freshness";

export const DEFAULT_K = 10;
export const MAX_K = 50;

export function clampK(raw: unknown): number {
  const n = typeof raw === "number" ? Math.floor(raw) : DEFAULT_K;
  return Math.min(MAX_K, Math.max(1, Number.isFinite(n) ? n : DEFAULT_K));
}

/** The slice of the Worker environment the search path needs. */
export interface SearchEnv {
  QUERY_CACHE: DurableObjectNamespace;
  /**
   * Hot-tier read deps (#13105). Absent by default — and no caller
   * provides them yet (tenant provisioning and the Turso
   * watermark source are follow-on work) — so the pre-cache path
   * below runs exactly as before. When present, a fresh Class B
   * row for the query key short-circuits the DO fan-out.
   */
  hotCache?: {
    reader: HotCacheReader;
    source: WatermarkSource;
    tenantId: string;
    nowSeconds?: () => number;
    onSkip?: (log: SkipLog) => void;
  };
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
  if (env.hotCache) {
    const hot = env.hotCache;
    const outcome = await tryServeFromHotCache({
      store: hot.reader,
      source: hot.source,
      tenantId: hot.tenantId,
      cacheKey: key,
      dataClass: "B",
      nowSeconds: hot.nowSeconds ?? (() => Math.floor(Date.now() / 1000)),
      onSkip: hot.onSkip,
    });
    if (outcome.served) {
      const body = JSON.parse(new TextDecoder().decode(outcome.entry.payload)) as Record<string, unknown>;
      return { status: 200, payload: { cache: "hot", key, query, ...body } };
    }
  }
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
