/**
 * Read-path hot-cache integration (#13105, follow-on 4 of 6 under
 * #9134; design §2 + §8 item 4).
 *
 * The §2 skip path as one composition: read the (tenant, key) row
 * from the D1 store (#13100), decide freshness for its class with
 * the pure evaluator (#13101), and serve the payload only inside
 * the bound. Every skip is logged with (tenant, key, class, entry
 * watermark, entry age) — the log is the measurement surface §7
 * aggregates, so logging is part of the path, not an afterthought.
 *
 * Failure posture (design §1 + §6, and the slice acceptance):
 * anything the tier cannot prove — a missing row, a D1 throw, a
 * wrong-class row, a stale verdict — resolves to "not served" and
 * the caller keeps the pre-cache path exactly. A D1 outage in
 * particular degrades to the pre-cache Turso path with no
 * error-surface change.
 *
 * Class A note: the D1 row carries no content-hash column (schema
 * is follow-on 1's, frozen), so the hash is computed over the
 * stored payload at read time and compared to the requested hash.
 * Class A hydration has no call site in this Worker yet — this
 * module is the decision primitive the hydration path will call;
 * only Class B (result sets) is wired into executeSearch so far.
 */
import {
  evaluateFreshness,
  readTenantWatermark,
  type DataClass,
  type FreshnessReason,
} from "./freshness";
import type { HotCacheEntry } from "./hot_cache";

/**
 * The storage half of the seam: the structural surface of
 * HotCacheStore this path needs, so tests can substitute a
 * throwing store for the D1-outage case without a real outage.
 */
export interface HotCacheReader {
  get(tenantId: string, cacheKey: string): Promise<HotCacheEntry | undefined>;
  touch(tenantId: string, cacheKey: string, atSeconds: number): Promise<void>;
}

export type { HotCacheEntry };

/** One logged skip: the §2 measurement shape plus the verdict reason. */
export interface SkipLog {
  tenant: string;
  key: string;
  dataClass: DataClass;
  entryWatermark: string;
  entryAge: number;
  reason: FreshnessReason;
  degraded: boolean;
}

/** Why a read was not served from the tier. */
export type HotCacheMissReason =
  /** No row for the (tenant, key) pair. */
  | "absent"
  /** The D1 read threw: outage, fall through with no error-surface change. */
  | "store-unavailable"
  /** The row's class is not the class the read asked for. */
  | "class-mismatch"
  /** The evaluator's stale verdict (never a serve). */
  | FreshnessReason;

export type HotCacheReadOutcome =
  | { served: true; entry: HotCacheEntry; log: SkipLog }
  | { served: false; reason: HotCacheMissReason };

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes as unknown as ArrayBuffer);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

export interface HotCacheReadArgs {
  store: HotCacheReader;
  /** The tenant's Turso watermark source; never called for Class A reads. */
  source: (tenantId: string) => Promise<string | null>;
  tenantId: string;
  cacheKey: string;
  dataClass: DataClass;
  /** Class A only: the content hash the request names. */
  requestedContentHash?: string;
  /** Class C only: latest invalidation event time, unix seconds. */
  lastInvalidationAt?: number | null;
  nowSeconds: () => number;
  onSkip?: (log: SkipLog) => void;
}

/**
 * Try one hot-cache serve. Pure decision, injected I/O: the only
 * awaits are the store read, the watermark read, and the
 * best-effort hit touch. Never throws for tier conditions — a
 * throwing store resolves to `store-unavailable`, never to an
 * error the caller must translate.
 */
export async function tryServeFromHotCache(args: HotCacheReadArgs): Promise<HotCacheReadOutcome> {
  const { store, source, tenantId, cacheKey, dataClass, nowSeconds } = args;
  let entry: HotCacheEntry | undefined;
  try {
    entry = await store.get(tenantId, cacheKey);
  } catch {
    return { served: false, reason: "store-unavailable" };
  }
  if (!entry) return { served: false, reason: "absent" };
  if (entry.entryClass !== dataClass) return { served: false, reason: "class-mismatch" };

  const now = nowSeconds();
  // Class A (when the request names no hash) and Class C (when no
  // invalidation event is known) evaluate without extra reads; the
  // watermark read below is skipped entirely for Class A reads.
  const watermark =
    dataClass === "A" ? { status: "ok" as const, currentWatermark: entry.sourceWatermark } : await readTenantWatermark(source, tenantId);
  const verdict = evaluateFreshness(
    {
      dataClass,
      sourceWatermark: entry.sourceWatermark,
      filledAt: entry.filledAt,
      contentHash:
        dataClass === "A" && args.requestedContentHash !== undefined ? await sha256Hex(entry.payload) : undefined,
    },
    {
      now,
      watermark,
      requestedContentHash: args.requestedContentHash,
      lastInvalidationAt: args.lastInvalidationAt ?? null,
    },
  );
  if (!verdict.fresh) return { served: false, reason: verdict.reason };

  // Best effort: a touch failure must not fail a serve the
  // evaluator already approved.
  try {
    await store.touch(tenantId, cacheKey, now);
  } catch {
    // Hit bookkeeping is measurement, not correctness.
  }
  const log: SkipLog = {
    tenant: tenantId,
    key: cacheKey,
    dataClass,
    entryWatermark: entry.sourceWatermark,
    entryAge: verdict.ageSeconds,
    reason: verdict.reason,
    degraded: verdict.degraded,
  };
  args.onSkip?.(log);
  return { served: true, entry, log };
}
