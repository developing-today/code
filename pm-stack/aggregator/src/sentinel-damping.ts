// Flap damping for the stack sentinel (#9049): the sentinel's probe
// loop used to flip a service to "down" — and post the transition to
// the alert throat — on a SINGLE failed probe. Under load a healthy
// service's probe times out, the state flips down, recovers a round
// later, and every flip that outlasts the notification cooldown pages
// Drew. On 2026-11-12 the projector alone produced 118 notification
// events (4 sent) without its process ever crashing.
//
// The rule this module folds into the sentinel's per-service state:
// a service becomes "down" only after SENTINEL_FAILURE_THRESHOLD
// consecutive failed probes (at the sentinel's 30s cadence, ~90s of
// continuous failure). A success resets the streak. Recovery flips
// back on the first success, and — via the postedState bookkeeping
// the sentinel already kept — a recovery is only ever posted when
// the corresponding down was actually posted: a blip that never
// became a posted down produces no recovery, and a down whose post
// never landed (throat unreachable) also recovers silently.
//
// Pure state fold, pinned by fixture tests (test/sentinel-damping.test.ts);
// scripts/stack-sentinel.ts owns the probing, posting and persistence.

export type SentinelStateName = "down" | "up";

export interface SentinelServiceEntry {
  state: SentinelStateName;
  /** When the service entered this state (ISO). For a damped down,
   * this is when the failure streak began, not the flip round — the
   * outage started when the probes started failing. */
  since: string;
  /** The state last successfully posted to the throat (null = never). */
  postedState: SentinelStateName | null;
  detail?: string;
  /** Consecutive failed probes in the current not-yet-down streak. */
  pendingFailures: number;
  /** When the current failure streak began (ISO), or null. */
  pendingSince: string | null;
}

export interface SentinelProbeVerdict {
  state: SentinelStateName;
  detail: string;
}

/** Failed probes in a row required before a service is declared down. */
export const SENTINEL_FAILURE_THRESHOLD = 3;

/** A transition post is owed exactly when the posted state lags the state. */
export function sentinelPostOwed(entry: SentinelServiceEntry): boolean {
  return entry.postedState !== entry.state;
}

/**
 * Read a persisted entry (possibly written before #9049, i.e. without
 * the streak fields) into the current shape. Returns null for values
 * that are not a recognizable entry.
 */
export function normalizeSentinelEntry(value: unknown): SentinelServiceEntry | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const v = value as Record<string, unknown>;
  if (v.state !== "down" && v.state !== "up") return null;
  if (typeof v.since !== "string" || !v.since) return null;
  return {
    state: v.state,
    since: v.since,
    postedState: v.postedState === "down" || v.postedState === "up" ? v.postedState : null,
    ...(typeof v.detail === "string" ? { detail: v.detail } : {}),
    pendingFailures:
      typeof v.pendingFailures === "number" && Number.isFinite(v.pendingFailures) && v.pendingFailures > 0
        ? Math.floor(v.pendingFailures)
        : 0,
    pendingSince: typeof v.pendingSince === "string" && v.pendingSince ? v.pendingSince : null,
  };
}

/**
 * Fold one probe verdict into a service's entry. `prev` undefined is
 * the first sighting: an up verdict baselines silently (postedState
 * starts at "up", so nothing is owed); a down verdict starts the
 * failure streak instead of posting immediately — a genuinely down
 * service is still declared after `threshold` probes (~90s).
 */
export function applySentinelProbe(
  prev: SentinelServiceEntry | undefined,
  verdict: SentinelProbeVerdict,
  now: string,
  threshold: number = SENTINEL_FAILURE_THRESHOLD,
): SentinelServiceEntry {
  const base: SentinelServiceEntry = prev ?? {
    state: "up",
    since: now,
    postedState: "up",
    pendingFailures: 0,
    pendingSince: null,
  };
  if (verdict.state === "up") {
    if (base.state === "down") {
      // Recovery: flip on the first success. Whether the recovery is
      // posted is decided by postedState (sentinelPostOwed) — only a
      // down that was actually posted earns a recovery post.
      return {
        state: "up",
        since: now,
        postedState: base.postedState,
        detail: verdict.detail,
        pendingFailures: 0,
        pendingSince: null,
      };
    }
    // Still up: any failure streak is broken.
    return { ...base, detail: verdict.detail, pendingFailures: 0, pendingSince: null };
  }
  // Verdict: down.
  if (base.state === "down") {
    // Already down: refresh the detail, owe nothing new.
    return { ...base, detail: verdict.detail, pendingFailures: 0, pendingSince: null };
  }
  const pendingFailures = base.pendingFailures + 1;
  const pendingSince = base.pendingSince ?? now;
  if (pendingFailures >= threshold) {
    return {
      state: "down",
      since: pendingSince,
      postedState: base.postedState,
      detail: verdict.detail,
      pendingFailures: 0,
      pendingSince: null,
    };
  }
  return {
    ...base,
    detail: `probe failing ${pendingFailures}/${threshold}: ${verdict.detail}`,
    pendingFailures,
    pendingSince,
  };
}
