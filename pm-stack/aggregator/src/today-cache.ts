// The /today short-TTL cache (#1179).
//
// todayContext() fans out to board, roadmap, attention, health/quota,
// and orchestration on EVERY /today?full=1 and /frag/today request, and
// the /today shell's /frag/today load means a single page view costs
// that fan-out twice in quick succession (shell signals + fragment).
// In live mode those are Kaneo/Grist/projector reads (~1.8s measured
// on #1179), so repeated warm refreshes and concurrent cold loads were
// all paying the full fan-out.
//
// This module bounds that repeated work: a small TTL (10s, inside the
// issue's proposed 5-15s window) serves warm hits from the last
// successful read, and concurrent cold requests share one in-flight
// load instead of each fanning out.
//
// Truthfulness notes (the freshness-sensitive part of #1179):
// - Only successful reads are cached, and only the DERIVED payload
//   (board/roadmap/attention/health/quota views). The per-request
//   provenance strings (source.provenance()) and detail
//   (provenanceDetail(), with its feed-age computation) are still read
//   live on every request in server.ts, so the provenance STATE
//   (live/fixtures/stale/…) never reflects cache time.
// - Ages inside the cached payload (feed-age text, the orchestration
//   section's generatedAt) are stamped at read time and can lag by at
//   most the TTL — the same bound the health probe cache (60s, #1147)
//   already accepts, at a 6x tighter budget.
// - Fail-soft fallbacks (empty quota view + quotaUnavailable, null
//   health) are cached as-is: they ARE the honest degraded state of
//   that read, and expiry (or a loader throw, which is never cached)
//   triggers a fresh read on the next request.

export const TODAY_CACHE_TTL_MS = 10_000;

export class TodayCache<T> {
  private cached: { at: number; data: T } | null = null;
  private inflight: Promise<T> | null = null;

  constructor(
    private readonly ttlMs: number = TODAY_CACHE_TTL_MS,
    private readonly now: () => number = Date.now,
  ) {}

  /** Forget the cached value and any shared in-flight load (tests). */
  clear(): void {
    this.cached = null;
    this.inflight = null;
  }

  /**
   * Return the cached value on a warm hit, the shared in-flight load
   * when a cold load is already running, or run `loader` and cache a
   * successful result. A throwing loader never poisons the cache: the
   * in-flight slot is released and the error rethrows, so the next
   * request retries (and server.ts's error boundary still 500s).
   */
  get(loader: () => Promise<T>): Promise<T> {
    const at = this.now();
    if (this.cached && at - this.cached.at < this.ttlMs) {
      return Promise.resolve(this.cached.data);
    }
    this.inflight ??= loader().then(
      (data) => {
        this.cached = { at: this.now(), data };
        this.inflight = null;
        return data;
      },
      (err) => {
        this.inflight = null;
        throw err;
      },
    );
    return this.inflight;
  }
}
