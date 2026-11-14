/**
 * #3976: the SSE-down degraded-mode policy for browser clients.
 *
 * The #3978 stream is the fast path: a notice lands, the client rereads
 * canonical JSON/page data, the view updates in place. When the stream
 * is down (EventSource error / closed), the client must not sit on
 * stale data forever and must not spin either — the design's degraded
 * mode is the collector's own cadence: refresh canonical data every
 * five minutes while the stream is down, and stop the moment the
 * stream is back. Refreshing means re-issuing the fragment/JSON reads
 * (the `dashrefresh` path), never a full page reload, so view state
 * (scroll, form input, filters) survives the fallback exactly as it
 * survives a stream-driven refresh.
 *
 * This module is the pure policy the browser asset
 * (src/assets/freshness-client.js) implements; it lives here so the
 * cadence and the resume semantics are unit-tested, not re-derived in
 * an untested script.
 */

export const SSE_DOWN_REFRESH_MS = 5 * 60_000;

export interface PushFallbackTracker {
  /** When the stream most recently went down; null while it is live. */
  streamDownAtMs: number | null;
  /** Last fallback (or stream-driven) canonical refresh. */
  lastRefreshAtMs: number;
}

export function newPushFallbackTracker(nowMs: number): PushFallbackTracker {
  return { streamDownAtMs: null, lastRefreshAtMs: nowMs };
}

/** The stream (re)connected: degraded mode ends immediately, whatever
 *  the fallback timer was about to do. */
export function noteStreamOpen(tracker: PushFallbackTracker): PushFallbackTracker {
  return { streamDownAtMs: null, lastRefreshAtMs: tracker.lastRefreshAtMs };
}

/** The stream dropped. The first drop starts the cadence clock; a
 *  repeated error while already down does not restart it (EventSource
 *  retries on its own and fires error repeatedly). */
export function noteStreamDown(tracker: PushFallbackTracker, nowMs: number): PushFallbackTracker {
  if (tracker.streamDownAtMs !== null) return tracker;
  return { streamDownAtMs: nowMs, lastRefreshAtMs: tracker.lastRefreshAtMs };
}

export function noteCanonicalRefresh(tracker: PushFallbackTracker, nowMs: number): PushFallbackTracker {
  return { streamDownAtMs: tracker.streamDownAtMs, lastRefreshAtMs: nowMs };
}

/** Is a degraded-mode canonical refresh due at `nowMs`? Due exactly on
 *  the 5-minute cadence measured from the later of (stream went down,
 *  last refresh) — never while the stream is live. */
export function fallbackRefreshDue(tracker: PushFallbackTracker, nowMs: number): boolean {
  if (tracker.streamDownAtMs === null) return false;
  const anchor = Math.max(tracker.streamDownAtMs, tracker.lastRefreshAtMs);
  return nowMs - anchor >= SSE_DOWN_REFRESH_MS;
}

/** Milliseconds until the next fallback refresh is due; null while the
 *  stream is live (no fallback timer should be running). */
export function fallbackRefreshDelayMs(tracker: PushFallbackTracker, nowMs: number): number | null {
  if (tracker.streamDownAtMs === null) return null;
  const anchor = Math.max(tracker.streamDownAtMs, tracker.lastRefreshAtMs);
  return Math.max(0, anchor + SSE_DOWN_REFRESH_MS - nowMs);
}
