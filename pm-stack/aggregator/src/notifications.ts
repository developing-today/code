// Notification measurement (#3319, slice (e) of the #1254 design):
// the weekly sent-vs-acted-on roll-up over the alert throat's send
// ledger (alerts.notification_sends, written by AlertsStore.logSend).
//
// The roll-up is a pure derivation over ledger rows so it can be
// pinned by fixture tests without a database; the /notifications page
// and its /api twin render exactly this shape.
//
// Counting rules (the page states them in plain words too):
//   - Only outcome = "sent" rows are sends. suppressed-quiet,
//     suppressed-dedupe, gone, and error rows are attempts that never
//     reached a device; they never enter the denominator, and a send
//     is never inferred from delivered/escalated incident events.
//   - A send counts as acted-on when its acted_on_at (stamped by the
//     exactly-once Ack transition in AlertsStore.ackIncident) falls
//     within 24 hours of the send. Ack is the durable v1 acted-on
//     signal; acting through any other surface is not attributable
//     to a specific send (see the page's attribution note).
//   - Weeks are rolling 7-day buckets ending at `now`, four of them:
//     the window is the last 28 days, not calendar weeks.
//   - A class whose rolling-window acted-on ratio is under 10% (with
//     at least one send) is flagged as a default-off PROPOSAL. The
//     flag is display-only: nothing here mutates a preference or a
//     default; default changes are Drew's call (design Q1).

import type { AttentionView } from "./derive.ts";
import type { HealthView } from "./health.ts";
import type { QuotaView, TierItem, TieredEntry } from "./model.ts";
import type { NotificationSendRow } from "./alerts.ts";

export const NOTIFICATION_CLASS_DEFAULTS: Record<string, boolean> = {
  incident: true,
  "needs-drew": true,
  "quota-wall": true,
  "quota-near-wall": false,
  "stack-health": true,
  "blocked-age": true,
};

export const NOTIFICATION_COOLDOWN_MS = 6 * 60 * 60_000;
export const BLOCKED_AGE_REMINDER_MS = 24 * 60 * 60_000;
export const BLOCKED_AGE_BREAKTHROUGH_MS = 48 * 60 * 60_000;
export const QUOTA_WELL_BEFORE_RESET_MS = 24 * 60 * 60_000;

export interface NotificationCandidate {
  class: "needs-drew" | "blocked-age" | "quota-wall" | "quota-near-wall" | "stack-health";
  objectRef: string;
  stateFingerprint: string;
  title: string;
  body: string;
  url: string;
  observedAt: string;
  breakthrough: boolean;
  digestOnly?: boolean;
  escalationStep?: 24 | 48;
}

export interface NotificationHistoryEntry {
  at: string;
  class: NotificationCandidate["class"];
  objectRef: string;
  stateFingerprint: string;
  outcome: "sent" | "suppressed-quiet" | "suppressed-dedupe" | "suppressed-cooldown" | "gone" | "error";
  subscriptionId?: string;
}

export type NotificationDecision = "send" | "digest" | "suppress-dedupe" | "suppress-cooldown";

export interface NotificationSnapshot {
  attention: AttentionView;
  quota: QuotaView;
  health: Pick<HealthView, "services">;
  /** Pool ids identified as having no alternative route. */
  soleDoorPoolIds?: ReadonlySet<string>;
}

function candidate(
  value: Omit<NotificationCandidate, "stateFingerprint" | "breakthrough"> & {
    state: unknown;
    breakthrough?: boolean;
  },
): NotificationCandidate {
  const { state, breakthrough = false, ...rest } = value;
  return { ...rest, stateFingerprint: JSON.stringify(state), breakthrough };
}

function tierItemCandidate(item: TierItem | TieredEntry, now: number): NotificationCandidate[] {
  const isTierItem = "id" in item;
  const sessionId = item.sessionId;
  const itemId = isTierItem ? item.id : sessionId;
  const itemRef = isTierItem ? item.ref : undefined;
  const objectRef = sessionId ? `session:${sessionId}` : `item:${itemId}`;
  const url = sessionId
    ? `/sessions?id=${encodeURIComponent(sessionId)}`
    : itemRef && /^[-\w.]+\/[-\w.]+#\d+$/.test(itemRef)
      ? `https://github.com/${itemRef.replace("#", "/issues/")}`
      : `/attention#${encodeURIComponent(itemId ?? "unknown")}`;
  const observedAt = isTierItem ? item.since : item.tierSince ?? item.createdAt ?? item.lastActivityAt ?? new Date(now).toISOString();
  const entry = candidate({
    class: "needs-drew",
    objectRef,
    title: item.title ?? itemId ?? "Needs-Drew item",
    body: "This item entered the needs-Drew tier.",
    url,
    observedAt,
    state: { tier: "needs-drew", state: "state" in item ? item.state : undefined, kind: isTierItem ? item.kind : undefined },
  });
  const since = Date.parse(observedAt);
  if (!Number.isFinite(since) || now - since < BLOCKED_AGE_REMINDER_MS) return [entry];
  const step: 24 | 48 = now - since >= BLOCKED_AGE_BREAKTHROUGH_MS ? 48 : 24;
  return [
    entry,
    candidate({
      class: "blocked-age",
      objectRef,
      title: step === 48 ? `Blocked for 48 hours: ${item.title ?? itemId}` : `Needs attention: ${item.title ?? itemId}`,
      body: "This needs-Drew item remains unacted.",
      url,
      observedAt,
      state: { step },
      breakthrough: step === 48,
      digestOnly: step === 24,
      escalationStep: step,
    }),
  ];
}

/** Apply enter-state dedupe, the per-item cooldown, and quiet/digest policy. */
export function decideNotification(
  candidate: NotificationCandidate,
  history: NotificationHistoryEntry[],
  now: number,
  quietHours: boolean,
): NotificationDecision {
  const sent = history.filter((entry) => entry.outcome === "sent");
  if (
    sent.some(
      (entry) =>
        entry.class === candidate.class &&
        entry.objectRef === candidate.objectRef &&
        entry.stateFingerprint === candidate.stateFingerprint,
    )
  ) {
    return "suppress-dedupe";
  }
  const lastForItem = sent
    .filter((entry) => entry.class === candidate.class && entry.objectRef === candidate.objectRef)
    .map((entry) => Date.parse(entry.at))
    .filter(Number.isFinite)
    .sort((a, b) => b - a)[0];
  const escalation = candidate.escalationStep !== undefined;
  if (!escalation && lastForItem !== undefined && now - lastForItem < NOTIFICATION_COOLDOWN_MS) {
    return "suppress-cooldown";
  }
  if (candidate.digestOnly || (quietHours && !candidate.breakthrough)) return "digest";
  return "send";
}

/** Derive stable, fixture-testable notifications from the same views the UI serves. */
export function buildNotificationCandidates(snapshot: NotificationSnapshot, now = Date.now()): NotificationCandidate[] {
  const attentionItems: Array<TierItem | TieredEntry> = [
    ...snapshot.attention.drew,
    ...snapshot.attention.drewExtra,
  ];
  const candidates = attentionItems.flatMap((item) => tierItemCandidate(item, now));

  for (const pool of snapshot.quota.pools) {
    const soleDoor = snapshot.soleDoorPoolIds?.has(pool.id) ?? false;
    const windows = pool.windows.filter((w) => w.limit !== null);
    const wall = pool.state === "walled" || windows.some((w) => (w.percent ?? (w.used / (w.limit || 1)) * 100) >= 100);
    if (wall) {
      const resetAt = windows.map((w) => Date.parse(w.resetsAt ?? "")).filter(Number.isFinite).sort((a, b) => a - b)[0];
      const wellBeforeReset = resetAt !== undefined && resetAt - now >= QUOTA_WELL_BEFORE_RESET_MS;
      if (soleDoor || wellBeforeReset) {
        candidates.push(
          candidate({
            class: "quota-wall",
            objectRef: `quota:${pool.id}`,
            title: `Quota wall: ${pool.provider} ${pool.plan}`,
            body: `Pool ${pool.id} is walled${soleDoor ? " and has no alternative route" : " well before reset"}.`,
            url: "/quota",
            observedAt: new Date(now).toISOString(),
            state: { state: "walled", resetAt: Number.isFinite(resetAt) ? new Date(resetAt).toISOString() : null },
            breakthrough: soleDoor,
          }),
        );
      }
    } else if (soleDoor && windows.some((w) => (w.percent ?? 0) >= 85 && (w.percent ?? 0) < 100)) {
      candidates.push(
        candidate({
          class: "quota-near-wall",
          objectRef: `quota:${pool.id}`,
          title: `Quota near wall: ${pool.provider} ${pool.plan}`,
          body: `Sole-door pool ${pool.id} has crossed 85%.`,
          url: "/quota",
          observedAt: new Date(now).toISOString(),
          state: { state: "near", windows: windows.map((w) => [w.name, Math.floor(w.percent ?? 0)]) },
        }),
      );
    }
  }

  for (const service of snapshot.health.services) {
    if (service.ok !== false) continue;
    candidates.push(
      candidate({
        class: "stack-health",
        objectRef: `health:${service.def.id}`,
        title: `Stack health: ${service.def.name}`,
        body: "The monitored service is failing.",
        url: "/health",
        observedAt: service.probedAt ?? new Date(now).toISOString(),
        state: { state: service.state },
      }),
    );
  }
  return candidates;
}

export const ACTED_WINDOW_MS = 24 * 60 * 60 * 1000;
export const ROLLING_WEEKS = 4;
export const WEEK_MS = 7 * 24 * 60 * 60 * 1000;
export const PROPOSAL_THRESHOLD = 0.1;

/** The send's timestamp: the ledger's insert-time column. Rows the
 * store writes carry no explicit timestamp, so the row's defaulted
 * created_at is the send time; sent_at wins when a row carries one. */
export function sendTimestamp(row: NotificationSendRow): string | null {
  return row.sent_at ?? row.created_at ?? null;
}

export interface NotificationWeekRollup {
  /** Inclusive bucket start (ISO); the bucket is [start, start + 7d). */
  start: string;
  sent: number;
  acted: number;
  /** acted / sent, or null when nothing was sent that week. */
  ratio: number | null;
}

export interface NotificationClassRollup {
  class: string;
  /** Oldest week first; always ROLLING_WEEKS entries. */
  weeks: NotificationWeekRollup[];
  sent: number;
  acted: number;
  /** acted / sent over the whole rolling window; null when sent = 0. */
  ratio: number | null;
  /** Display-only default-off proposal (ratio under the threshold). */
  proposal: boolean;
}

export interface NotificationRollup {
  /** The instant the rolling window ends (ISO). */
  asOf: string;
  /** Inclusive window start (ISO): asOf - ROLLING_WEEKS weeks. */
  windowStart: string;
  classes: NotificationClassRollup[];
  /** Ledger rows skipped because they carry no usable send timestamp. */
  undated: number;
}

export function rollupNotificationSends(
  rows: NotificationSendRow[],
  now: Date,
  weeks: number = ROLLING_WEEKS,
): NotificationRollup {
  const asOfMs = now.getTime();
  const windowStartMs = asOfMs - weeks * WEEK_MS;
  const byClass = new Map<string, { sent: number[]; acted: number[] }>();
  let undated = 0;
  for (const row of rows) {
    if (row.outcome !== "sent") continue;
    const tsRaw = sendTimestamp(row);
    const ts = tsRaw ? Date.parse(tsRaw) : Number.NaN;
    if (!Number.isFinite(ts)) {
      undated += 1;
      continue;
    }
    if (ts < windowStartMs || ts > asOfMs) continue;
    // Bucket 0 is the oldest week in the window.
    const bucket = Math.min(weeks - 1, Math.floor((ts - windowStartMs) / WEEK_MS));
    let entry = byClass.get(row.class);
    if (!entry) {
      entry = { sent: new Array<number>(weeks).fill(0), acted: new Array<number>(weeks).fill(0) };
      byClass.set(row.class, entry);
    }
    entry.sent[bucket] += 1;
    if (row.acted_on_at) {
      const actedMs = Date.parse(row.acted_on_at);
      if (Number.isFinite(actedMs) && actedMs >= ts && actedMs - ts <= ACTED_WINDOW_MS) {
        entry.acted[bucket] += 1;
      }
    }
  }
  const classes: NotificationClassRollup[] = [...byClass.entries()]
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([cls, entry]) => {
      const weekRollups: NotificationWeekRollup[] = entry.sent.map((sent, i) => ({
        start: new Date(windowStartMs + i * WEEK_MS).toISOString(),
        sent,
        acted: entry.acted[i],
        ratio: sent > 0 ? entry.acted[i] / sent : null,
      }));
      const sent = entry.sent.reduce((a, b) => a + b, 0);
      const acted = entry.acted.reduce((a, b) => a + b, 0);
      const ratio = sent > 0 ? acted / sent : null;
      return {
        class: cls,
        weeks: weekRollups,
        sent,
        acted,
        ratio,
        proposal: ratio !== null && ratio < PROPOSAL_THRESHOLD,
      };
    });
  return {
    asOf: now.toISOString(),
    windowStart: new Date(windowStartMs).toISOString(),
    classes,
    undated,
  };
}
