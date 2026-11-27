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
import { CEILING, CONTEXT, type Ceiling, type DisclosureContext } from "./space";
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

/**
 * A Durable Object answer as a JSON object, or null when it is not one
 * (#7983 item 5). The DO is first-party, but its answers are spread into
 * the route's own envelope and their fields read, so a body that is not
 * an object (or not JSON) has to become this route's error envelope
 * rather than a thrown TypeError or a payload built from garbage.
 */
export async function doJsonObject(resp: Response): Promise<Record<string, unknown> | null> {
  let value: unknown;
  try {
    value = await resp.json();
  } catch {
    return null;
  }
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/** The error payload for a Durable Object answer that failed its shape check. */
export function malformedDoPayload(route: string): Record<string, unknown> {
  return { error: { code: "upstream_malformed", message: `query cache returned a malformed ${route} response` } };
}

export interface SearchOutcome {
  status: number;
  payload: Record<string, unknown>;
}

/**
 * Run one search. `query` must already be normalized and
 * non-empty (the callers' validation owns that); `k` must
 * already be clamped. The ceiling/context default to the
 * surface pins (the REST surface always searches under them);
 * the MCP surface passes its caller's policy ceiling. On success
 * the payload is the REST envelope `{ cache, key, query, result,
 * meta }`; on an upstream failure it is the Durable Object's
 * error payload, with its status.
 */
export async function executeSearch(
  env: SearchEnv,
  query: string,
  k: number,
  opts?: { ceiling?: Ceiling; context?: DisclosureContext },
): Promise<SearchOutcome> {
  const ceiling = opts?.ceiling ?? CEILING;
  const context = opts?.context ?? CONTEXT;
  const key = await queryKey({ q: query, k, ceiling, context });
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
    body: JSON.stringify({ query, k, ceiling, context }),
  });
  const payload = await doJsonObject(resp);
  if (payload === null) return { status: 502, payload: malformedDoPayload("query") };
  if (!resp.ok) return { status: resp.status, payload };
  const { outcome, ...rest } = payload;
  if (outcome !== "hit" && outcome !== "miss") return { status: 502, payload: malformedDoPayload("query") };
  return { status: 200, payload: { cache: outcome, key, query, ...rest } };
}
