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
import type { NotificationSendRow } from "./alerts.ts";

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
