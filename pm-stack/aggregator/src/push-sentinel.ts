// The dead-push sentinel (#14084 S2, §3.5; lands #14048's detection ask).
//
// When delivery goes dark nothing can push about it — a push about
// no-push is a dead letter by design. So the sentinel raises stack
// incidents on the board and attention surfaces, never as a push:
// `warning` severity, which the engine never delivers (delivery is
// `critical`-only; "Warnings never push in v1").
//
// Three trip keys, each an independent incident opened by the engine
// tick and closed when its condition clears:
//   - `zero-endpoints`: the registry holds no live endpoints while
//     undelivered incidents exist.
//   - `no-successful-send`: no successful send in STALE_AFTER_DAYS
//     (v1: 2) while incidents are open.
//   - `undated-rows`: unified-ledger rows the roll-up cannot date —
//     the schema requires a writer-stamped `sent_at`, so an undatable
//     row is a data defect that pages here.
//
// The evaluator is pure (unit-tested); the tick helper below applies
// it through a structural store interface so it needs no import from
// alerts.ts (which imports this module for the engine step).
import { normalizeSendTimestamp } from "./notifications.ts";
import type { UnifiedSendRow } from "./push-registry.ts";

/** The sentinel's incident source. */
export const PUSH_SENTINEL_SOURCE = "push-sentinel";

export const ZERO_ENDPOINTS_KEY = "zero-endpoints";
export const STALE_SENDS_KEY = "no-successful-send";
export const UNDATED_ROWS_KEY = "undated-rows";

/** v1: N = 2 days without a successful send, with incidents open. */
export const STALE_AFTER_DAYS = 2;
export const STALE_AFTER_MS = STALE_AFTER_DAYS * 24 * 60 * 60 * 1000;

export type PushSentinelTrip = typeof ZERO_ENDPOINTS_KEY | typeof STALE_SENDS_KEY | typeof UNDATED_ROWS_KEY;

export interface PushSentinelSnapshot {
  /** Live endpoints in the registry. */
  liveEndpoints: number;
  /** Open incidents with no delivery yet. */
  openUndelivered: number;
  /** Open incidents of any delivery state. */
  incidentsOpen: number;
  /** Newest outcome=`sent` unified timestamp, if any. */
  lastSuccessAt: string | null;
  /** Unified-ledger rows the roll-up cannot date. */
  undated: number;
  nowMs: number;
}

export interface PushSentinelFinding {
  trip: PushSentinelTrip;
  title: string;
  body: string;
}

/** Pure evaluator: which sentinel conditions trip on this snapshot. */
export function evaluatePushSentinel(snap: PushSentinelSnapshot): PushSentinelFinding[] {
  const findings: PushSentinelFinding[] = [];
  if (snap.liveEndpoints === 0 && snap.openUndelivered > 0) {
    findings.push({
      trip: ZERO_ENDPOINTS_KEY,
      title: "Push has no live endpoints with undelivered incidents open",
      body:
        `The push registry holds zero live endpoints while ${snap.openUndelivered} incident(s) ` +
        `await delivery. Nothing can be delivered until an endpoint registers (re-subscribe under ` +
        `the one sender). Raised on the board, never as a push: a push about no-push is a dead letter.`,
    });
  }
  const lastMs = snap.lastSuccessAt ? Date.parse(snap.lastSuccessAt) : Number.NaN;
  if (snap.incidentsOpen > 0 && (!Number.isFinite(lastMs) || snap.nowMs - lastMs > STALE_AFTER_MS)) {
    findings.push({
      trip: STALE_SENDS_KEY,
      title: `No successful push in ${STALE_AFTER_DAYS} days with incidents open`,
      body:
        `${snap.incidentsOpen} incident(s) are open and the unified ledger shows no successful send ` +
        `within the last ${STALE_AFTER_DAYS} days. Delivery may be dark; check the sender and the ` +
        `endpoint census. Raised on the board, never as a push.`,
    });
  }
  if (snap.undated > 0) {
    findings.push({
      trip: UNDATED_ROWS_KEY,
      title: `${snap.undated} unified send-ledger row(s) carry no usable timestamp`,
      body:
        `The unified ledger's schema requires a writer-stamped sent_at, so a row the roll-up cannot ` +
        `date is a data defect in the writer path — investigate the send writer, not the roll-up. ` +
        `Raised on the board, never as a push.`,
    });
  }
  return findings;
}

/** The incident-store surface the tick needs (structural — no import). */
export interface PushSentinelIncidentStore {
  postIncident(input: {
    source: string;
    key: string;
    severity: "critical" | "warning" | "info";
    title: string;
    body: string;
  }): Promise<{ incident: unknown; created: boolean }>;
  closeOpenIncidentByKey(
    source: string,
    key: string,
    detail?: Record<string, unknown>,
  ): Promise<{ incident: unknown; transitioned: boolean }>;
}

export interface PushSentinelTickStore extends PushSentinelIncidentStore {
  listSubscriptions(): Promise<{ id: string }[]>;
  listUnifiedSends(): Promise<Pick<UnifiedSendRow, "outcome" | "sent_at">[]>;
  /** Open incidents: status, delivered_at (null until delivered). */
  listOpenIncidents(): Promise<{ status: string; delivered_at: string | null }[]>;
}

/**
 * Apply the evaluator: open a warning incident per tripped key, close
 * the incident for each cleared key. Never throws — the engine tick
 * calls this fail-soft and a sentinel failure must not break
 * delivery, escalation, or the dashboard tick.
 */
export async function runPushSentinelTick(
  store: PushSentinelTickStore,
  now: Date = new Date(),
): Promise<PushSentinelFinding[]> {
  try {
    const [subs, sends, open] = await Promise.all([
      store.listSubscriptions(),
      store.listUnifiedSends(),
      store.listOpenIncidents(),
    ]);
    const sentRows = sends.filter((r) => r.outcome === "sent");
    const lastMs = sentRows
      .map((r) => Date.parse(r.sent_at))
      .filter((ms) => Number.isFinite(ms))
      .reduce((max, ms) => Math.max(max, ms), Number.NaN);
    const undated = sentRows.filter((r) => !normalizeSendTimestamp(r.sent_at)).length;
    const findings = evaluatePushSentinel({
      liveEndpoints: subs.length,
      openUndelivered: open.filter((i) => i.status === "open" && !i.delivered_at).length,
      incidentsOpen: open.filter((i) => i.status === "open").length,
      lastSuccessAt: Number.isFinite(lastMs) ? new Date(lastMs).toISOString() : null,
      undated,
      nowMs: now.getTime(),
    });
    const tripped = new Set(findings.map((f) => f.trip));
    for (const finding of findings) {
      await store.postIncident({
        source: PUSH_SENTINEL_SOURCE,
        key: finding.trip,
        severity: "warning",
        title: finding.title,
        body: finding.body,
      });
    }
    for (const key of [ZERO_ENDPOINTS_KEY, STALE_SENDS_KEY, UNDATED_ROWS_KEY] as const) {
      if (!tripped.has(key)) {
        await store.closeOpenIncidentByKey(PUSH_SENTINEL_SOURCE, key, { reason: "sentinel condition cleared" });
      }
    }
    return findings;
  } catch (err) {
    console.warn(`push sentinel: tick failed (${err instanceof Error ? err.message : "error"})`);
    return [];
  }
}
