// The /today short-TTL cache (#1179), stale-while-revalidate since #5082.
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
// #5082 (2026-11-13): expiry used to make the NEXT request pay the
// full fan-out synchronously — the board/overview assemblies measure
// 3-8s cold, so an hourly watch's first pass (always past the 10s TTL)
// read 9-17s while its second pass read ~0s. The cache is now
// stale-while-revalidate, the same contract analytics-store.ts uses
// for the quota aggregates: an expired entry is served immediately
// and ONE background refresh (single-flight) re-reads behind it. The
// first load in a process is still awaited (coalesced) — server.ts
// warms the page contexts at boot so even that is off the request
// path in practice.
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
//   most the TTL plus one refresh — the same bound the health probe
//   cache (60s, #1147) already accepts, at a tighter budget.
// - Fail-soft fallbacks (empty quota view + quotaUnavailable, null
//   health) are cached as-is: they ARE the honest degraded state of
//   that read. A failed background refresh keeps serving the last
//   good value and the next expired read retries the refresh.
// - clear() still forces a full fresh read on the next request (the
//   freshness-notice invalidation in server.ts relies on it), and a
//   refresh started before a clear() can never repopulate the cache
//   behind it (generation guard).

export const TODAY_CACHE_TTL_MS = 10_000;

export class TodayCache<T> {
  private cached: { at: number; data: T } | null = null;
  private inflight: Promise<T> | null = null;
  private refreshing = false;
  private generation = 0;

  constructor(
    private readonly ttlMs: number = TODAY_CACHE_TTL_MS,
    private readonly now: () => number = Date.now,
  ) {}

  /** Forget the cached value and any shared in-flight load (tests). */
  clear(): void {
    this.cached = null;
    this.inflight = null;
    this.generation++;
  }

  /**
   * Warm hit: return the cached value. Expired hit: return the stale
   * value immediately and kick off a single-flight background refresh.
   * Cold miss: run `loader` (concurrent callers share the one load)
   * and cache a successful result. A throwing cold loader never
   * poisons the cache: the in-flight slot is released and the error
   * rethrows, so the next request retries (and server.ts's error
   * boundary still 500s).
   */
  get(loader: () => Promise<T>): Promise<T> {
    const at = this.now();
    if (this.cached && at - this.cached.at < this.ttlMs) {
      return Promise.resolve(this.cached.data);
    }
    if (this.cached) {
      this.refresh(loader);
      return Promise.resolve(this.cached.data);
    }
    const generation = this.generation;
    this.inflight ??= loader().then(
      (data) => {
        if (generation === this.generation) {
          this.cached = { at: this.now(), data };
        }
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

  /** Background re-read behind an expired entry; never throws. */
  private refresh(loader: () => Promise<T>): void {
    if (this.refreshing) return;
    this.refreshing = true;
    const generation = this.generation;
    loader().then(
      (data) => {
        if (generation === this.generation) {
          this.cached = { at: this.now(), data };
        }
        this.refreshing = false;
      },
      () => {
        // Keep serving the stale value; the next expired read retries.
        this.refreshing = false;
      },
    );
  }
}
