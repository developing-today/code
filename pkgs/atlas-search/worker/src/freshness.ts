/**
 * D1 hot-cache freshness evaluator (#13101; design #9134 §1, merged
 * via PR #13098). Follow-on 2 of 6 under #9134: the per-class
 * freshness rules, the tenant watermark read, and fail-safe
 * degradation. The cache store itself is follow-on 1 (#13100) and
 * read-path integration is follow-on 4; this module is the pure
 * decision core both call.
 *
 * A cached entry may be served only inside its freshness bound;
 * outside the bound the read revalidates against Turso. There is no
 * serve-stale mode. Freshness is per data class (§1):
 *
 *   A — content-addressed payloads. The content hash is the version:
 *       a body whose hash matches the request cannot be stale. No
 *       TTL, no watermark check; the watermark state is irrelevant.
 *   B — query result sets. Fresh iff the entry's source_watermark
 *       still equals the tenant's current watermark AND age ≤ TTL_B
 *       (default 60 s) — whichever fails first ends freshness, so a
 *       watermark advance makes an entry stale even inside the TTL.
 *   C — tenant aggregates. Fresh iff age ≤ TTL_C (default 15 min)
 *       AND no invalidation event for the tenant post-dates the
 *       entry.
 *
 * Fail-safe degradation (§1): when the tenant's current watermark
 * cannot be read, unavailability of the truth source narrows the
 * bound and never widens it — Class B degrades to TTL-only with
 * TTL_B divided by the degraded divisor (default 2 → 30 s), Class C
 * stops being served, Class A is unaffected. A serve that violates
 * these rules is a defect, not a mode: §7's stale_serves counter
 * counts watermark-mismatch serves and its expected value is zero.
 *
 * Purity: evaluateFreshness does no I/O and reads no clock — the
 * caller supplies `now` and the watermark read result. The read
 * itself is the thin seam readTenantWatermark, which maps every
 * failure of the source (throw, null) to `unavailable` so a broken
 * truth source can only ever narrow the bound.
 */

/** The three data classes of design #9134 §1. */
export type DataClass = "A" | "B" | "C";

/**
 * Freshness bounds, named config values per design #9134 §1 (all
 * bounds are configuration, program §7). Defaults: TTL_B 60 s,
 * TTL_C 15 min, degraded divisor 2.
 */
export interface FreshnessConfig {
  /** Class B TTL in seconds. */
  ttlBSeconds: number;
  /** Class C TTL in seconds. */
  ttlCSeconds: number;
  /** Divisor applied to TTL_B when the watermark source is unreachable. */
  degradedDivisor: number;
}

export const DEFAULT_FRESHNESS_CONFIG: FreshnessConfig = {
  ttlBSeconds: 60,
  ttlCSeconds: 15 * 60,
  degradedDivisor: 2,
};

function positiveFiniteOr(value: number | undefined, fallback: number): number {
  return value !== undefined && Number.isFinite(value) && value > 0 ? value : fallback;
}

/**
 * Resolve a partial override against the defaults. An override is
 * honored only when it is a positive finite number, and the degraded
 * divisor is clamped to ≥ 1: a bad config value must never widen a
 * bound past its default or turn degradation into an extension.
 */
export function resolveFreshnessConfig(overrides?: Partial<FreshnessConfig>): FreshnessConfig {
  const divisor = positiveFiniteOr(overrides?.degradedDivisor, DEFAULT_FRESHNESS_CONFIG.degradedDivisor);
  return {
    ttlBSeconds: positiveFiniteOr(overrides?.ttlBSeconds, DEFAULT_FRESHNESS_CONFIG.ttlBSeconds),
    ttlCSeconds: positiveFiniteOr(overrides?.ttlCSeconds, DEFAULT_FRESHNESS_CONFIG.ttlCSeconds),
    degradedDivisor: Math.max(1, divisor),
  };
}

/** The fill-time facts every cached entry carries (design §1, §5). */
export interface CacheEntryFacts {
  dataClass: DataClass;
  /** The tenant's corpus watermark at the Turso source when the entry was filled. */
  sourceWatermark: string;
  /** Wall-clock fill time, unix seconds (the §5 schema's filled_at). */
  filledAt: number;
  /** Class A only: the content hash the stored body is addressed by. */
  contentHash?: string;
}

/**
 * The result of reading the tenant's current watermark: either the
 * watermark verbatim, or `unavailable` when the source cannot be
 * read. There is deliberately no third state — unknown is
 * unavailable, and unavailable only ever narrows the bound.
 */
export type WatermarkRead = { status: "ok"; currentWatermark: string } | { status: "unavailable" };

/** Everything the evaluator needs about the moment of the read. */
export interface FreshnessContext {
  /** Current wall-clock time, unix seconds (same clock domain as filledAt). */
  now: number;
  watermark: WatermarkRead;
  /**
   * Class A only: the content hash the request names. When supplied,
   * the entry is fresh iff its hash equals it. When omitted, the
   * entry is addressed by its hash by construction (the §5 key) and
   * is fresh.
   */
  requestedContentHash?: string;
  /**
   * The latest invalidation event time for the tenant (unix
   * seconds), or null when no event is known. Class C only: an
   * event that post-dates the entry makes it stale (§4 mechanism 2).
   */
  lastInvalidationAt?: number | null;
}

/** Why the evaluator reached its verdict — the §2 skip log's reason surface. */
export type FreshnessReason =
  /** A: the stored body's hash is the requested version. */
  | "content-hash-match"
  /** A: the stored body is not the requested version — never served. */
  | "content-hash-mismatch"
  /** B: watermark still current and inside TTL_B. */
  | "watermark-current"
  /** B: the tenant watermark advanced past the entry's fill watermark. */
  | "watermark-advanced"
  /** B (degraded): inside the halved TTL-only bound. */
  | "degraded-ttl"
  /** B/C: age exceeds the applicable TTL bound. */
  | "ttl-expired"
  /** C: inside TTL_C with no later invalidation event. */
  | "within-ttl"
  /** C: an invalidation event post-dates the entry. */
  | "invalidated"
  /** C: the watermark source is unreachable — aggregates are not served. */
  | "watermark-unavailable"
  /** B/C: filledAt is in the future — the age comparison cannot be trusted. */
  | "clock-skew";

export interface FreshnessVerdict {
  /** True only when the entry may be served without revalidation. */
  fresh: boolean;
  reason: FreshnessReason;
  /** Entry age at evaluation time, seconds (negative under clock skew). */
  ageSeconds: number;
  /** The TTL bound that applied, seconds; null for Class A (no TTL). */
  boundSeconds: number | null;
  /** True when the watermark source was unreachable for this evaluation. */
  degraded: boolean;
}

/**
 * The pure freshness decision of design §1. Never serves outside
 * the bound: every uncertainty (unreachable watermark, future fill
 * time, unprovable hash) resolves to not fresh or to a narrower
 * bound, never to a wider one.
 */
export function evaluateFreshness(
  entry: CacheEntryFacts,
  context: FreshnessContext,
  config: FreshnessConfig = DEFAULT_FRESHNESS_CONFIG,
): FreshnessVerdict {
  const ageSeconds = context.now - entry.filledAt;
  const degraded = context.watermark.status === "unavailable";

  if (entry.dataClass === "A") {
    // The hash is the version: no TTL, no watermark check, and the
    // watermark state (including its unavailability) is irrelevant.
    if (context.requestedContentHash !== undefined && entry.contentHash !== context.requestedContentHash) {
      return { fresh: false, reason: "content-hash-mismatch", ageSeconds, boundSeconds: null, degraded };
    }
    return { fresh: true, reason: "content-hash-match", ageSeconds, boundSeconds: null, degraded };
  }

  // Classes B and C compare ages against a TTL; a fill time in the
  // future makes that comparison meaningless, so fail safe.
  if (ageSeconds < 0) {
    return {
      fresh: false,
      reason: "clock-skew",
      ageSeconds,
      boundSeconds: entry.dataClass === "B" ? config.ttlBSeconds : config.ttlCSeconds,
      degraded,
    };
  }

  if (entry.dataClass === "B") {
    if (context.watermark.status === "unavailable") {
      // Degraded: TTL-only, halved (floored — rounding may only
      // narrow the bound), never extended.
      const boundSeconds = Math.floor(config.ttlBSeconds / Math.max(1, config.degradedDivisor));
      return ageSeconds <= boundSeconds
        ? { fresh: true, reason: "degraded-ttl", ageSeconds, boundSeconds, degraded }
        : { fresh: false, reason: "ttl-expired", ageSeconds, boundSeconds, degraded };
    }
    if (entry.sourceWatermark !== context.watermark.currentWatermark) {
      // A watermark advance makes the entry stale by rule, even
      // inside the TTL — the watermark is the primary, clock-free
      // guard (§6), so it is reported ahead of the TTL check.
      return { fresh: false, reason: "watermark-advanced", ageSeconds, boundSeconds: config.ttlBSeconds, degraded };
    }
    return ageSeconds <= config.ttlBSeconds
      ? { fresh: true, reason: "watermark-current", ageSeconds, boundSeconds: config.ttlBSeconds, degraded }
      : { fresh: false, reason: "ttl-expired", ageSeconds, boundSeconds: config.ttlBSeconds, degraded };
  }

  // Class C — tenant aggregates.
  if (context.watermark.status === "unavailable") {
    // Aggregates are the class most likely to be quoted as exact:
    // not served at all while the truth source is unreachable.
    return { fresh: false, reason: "watermark-unavailable", ageSeconds, boundSeconds: config.ttlCSeconds, degraded };
  }
  if (context.lastInvalidationAt != null && context.lastInvalidationAt > entry.filledAt) {
    return { fresh: false, reason: "invalidated", ageSeconds, boundSeconds: config.ttlCSeconds, degraded };
  }
  return ageSeconds <= config.ttlCSeconds
    ? { fresh: true, reason: "within-ttl", ageSeconds, boundSeconds: config.ttlCSeconds, degraded }
    : { fresh: false, reason: "ttl-expired", ageSeconds, boundSeconds: config.ttlCSeconds, degraded };
}

/**
 * A tenant watermark source: resolves the tenant's current corpus
 * watermark at the Turso source of truth, or null when it has none
 * on record. Implementations do the I/O; this seam is the only
 * impure part of the freshness path.
 */
export type WatermarkSource = (tenantId: string) => Promise<string | null>;

/**
 * The tenant watermark read (§8 item 2). Every failure of the
 * source — a throw, a rejection, a null — maps to `unavailable`, so
 * the evaluator degrades the bound instead of guessing.
 */
export async function readTenantWatermark(source: WatermarkSource, tenantId: string): Promise<WatermarkRead> {
  try {
    const currentWatermark = await source(tenantId);
    return currentWatermark === null ? { status: "unavailable" } : { status: "ok", currentWatermark };
  } catch {
    return { status: "unavailable" };
  }
}

/**
 * Convenience composition for the read path: read the tenant's
 * watermark through the source, then evaluate purely. Equivalent to
 * evaluateFreshness with the read's result as the context watermark.
 */
export async function evaluateWithWatermarkRead(
  entry: CacheEntryFacts,
  tenantId: string,
  source: WatermarkSource,
  context: Omit<FreshnessContext, "watermark">,
  config: FreshnessConfig = DEFAULT_FRESHNESS_CONFIG,
): Promise<FreshnessVerdict> {
  const watermark = await readTenantWatermark(source, tenantId);
  return evaluateFreshness(entry, { ...context, watermark }, config);
}
