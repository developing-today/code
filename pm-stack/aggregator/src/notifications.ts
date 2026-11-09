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
  "quota-near-wall": true,
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
  escalationStep?: 24 | 48 | "weekly";
  /** Dashboard object-ref aliases used to stop escalation after Drew acts. */
  actedRefs?: string[];
}

export interface NotificationHistoryEntry {
  at: string;
  class: NotificationCandidate["class"];
  objectRef: string;
  stateFingerprint: string;
  outcome:
    | "sent"
    | "suppressed-quiet"
    | "suppressed-dedupe"
    | "suppressed-cooldown"
    | "gone"
    | "error"
    | "queued"
    | "queue-cancelled"
    | "recovered";
  subscriptionId?: string;
  /** Queue events carry enough data to recover and deliver after restart. */
  candidate?: NotificationCandidate;
  dueAt?: string;
}

export type NotificationDecision = "send" | "digest" | "suppress-dedupe" | "suppress-cooldown";

export interface NotificationSnapshot {
  attention: AttentionView;
  quota: QuotaView;
  health: Pick<HealthView, "services">;
  /** Pool ids identified as having no alternative route. */
  soleDoorPoolIds?: ReadonlySet<string>;
  /** Drew-authored dashboard actions/comments, with their event times. */
  drewActivity?: Array<{ objectRef: string; at: string }>;
}

interface LocalClock {
  year: number;
  month: number;
  day: number;
  hour: number;
  minute: number;
}

function localClock(at: number, timeZone: string): LocalClock {
  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).formatToParts(new Date(at));
  const value = (type: Intl.DateTimeFormatPartTypes) => Number(parts.find((part) => part.type === type)?.value);
  return { year: value("year"), month: value("month"), day: value("day"), hour: value("hour"), minute: value("minute") };
}

function localTimeToInstant(clock: LocalClock, timeZone: string): number {
  const wanted = Date.UTC(clock.year, clock.month - 1, clock.day, clock.hour, clock.minute);
  let result = wanted;
  // Resolve the zone offset from the requested wall time. Digest times are
  // daytime boundaries (noon and quiet-window end), so they never land in
  // the repeated hour at a DST transition.
  for (let attempt = 0; attempt < 3; attempt += 1) {
    const seen = localClock(result, timeZone);
    const represented = Date.UTC(seen.year, seen.month - 1, seen.day, seen.hour, seen.minute);
    const correction = wanted - represented;
    result += correction;
    if (correction === 0) break;
  }
  return result;
}

/** Next per-subscription batch point: quiet-window end or local noon. */
export function nextNotificationDigestAt(
  now: number,
  quiet: { start: string; end: string; tz: string; enabled?: boolean },
): number {
  const clock = localClock(now, quiet.tz);
  const minute = clock.hour * 60 + clock.minute;
  const toMinute = (value: string) => {
    const [hour, min] = value.split(":").map(Number);
    return hour * 60 + min;
  };
  const start = toMinute(quiet.start);
  const end = toMinute(quiet.end);
  const inQuiet = quiet.enabled !== false && (start <= end ? minute >= start && minute < end : minute >= start || minute < end);
  const useEndToday = minute < end;
  const dayOffset = inQuiet ? (minute >= start && start > end ? 1 : 0) : minute < 12 * 60 ? 0 : useEndToday ? 0 : 1;
  const day = new Date(Date.UTC(clock.year, clock.month - 1, clock.day + dayOffset));
  const endDigest = inQuiet || minute >= 12 * 60;
  const targetClock: LocalClock = {
    year: day.getUTCFullYear(),
    month: day.getUTCMonth() + 1,
    day: day.getUTCDate(),
    hour: endDigest ? Number(quiet.end.slice(0, 2)) : 12,
    minute: endDigest ? Number(quiet.end.slice(3, 5)) : 0,
  };
  return localTimeToInstant(targetClock, quiet.tz);
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
  const observedAt = isTierItem ? item.since : item.tierSince ?? item.lastActivityAt ?? item.createdAt ?? new Date(now).toISOString();
  const actedRefs = [
    objectRef,
    ...(itemId ? [`orch:${itemId}`] : []),
    ...(itemRef ? [`gh:${itemRef}`] : []),
  ];
  const entry = candidate({
    class: "needs-drew",
    objectRef,
    title: item.title ?? itemId ?? "Needs-Drew item",
    body: "This item entered the needs-Drew tier.",
    url,
    observedAt,
    actedRefs,
    state: { tier: "needs-drew", state: "state" in item ? item.state : undefined, kind: isTierItem ? item.kind : undefined },
  });
  const since = Date.parse(observedAt);
  if (!Number.isFinite(since) || now - since < BLOCKED_AGE_REMINDER_MS) return [entry];
  const step: 24 | 48 = now - since >= BLOCKED_AGE_BREAKTHROUGH_MS ? 48 : 24;
  const reminders = [
    entry,
    candidate({
      class: "blocked-age",
      objectRef,
      title: step === 48 ? `Blocked for 48 hours: ${item.title ?? itemId}` : `Needs attention: ${item.title ?? itemId}`,
      body: "This needs-Drew item remains unacted.",
      url,
      observedAt,
      actedRefs,
      state: { step },
      breakthrough: step === 48,
      digestOnly: step === 24,
      escalationStep: step,
    }),
  ];
  if (step === 48) {
    const week = Math.floor((now - since - BLOCKED_AGE_BREAKTHROUGH_MS) / WEEK_MS);
    if (week > 0) {
      reminders.push(
        candidate({
          class: "blocked-age",
          objectRef,
          title: `Weekly reminder: ${item.title ?? itemId}`,
          body: `This needs-Drew item remains unacted (week ${week} after the 48-hour breakthrough).`,
          url,
          observedAt,
          actedRefs,
          state: { step: "weekly", week },
          digestOnly: true,
          escalationStep: "weekly",
        }),
      );
    }
  }
  return reminders;
}

/** Apply enter-state dedupe, the per-item cooldown, and quiet/digest policy. */
export function decideNotification(
  candidate: NotificationCandidate,
  history: NotificationHistoryEntry[],
  now: number,
  quietHours: boolean,
): NotificationDecision {
  const sent = history.filter((entry) => entry.outcome === "sent");
  // Episode-scoped dedupe (#7193 Codex P1): a "recovered" marker ends the
  // episode a send belonged to, so only sends after the latest recovery
  // for this class+object suppress a recurrence of the same state.
  const recoveredAt =
    history
      .filter((entry) => entry.outcome === "recovered" && entry.class === candidate.class && entry.objectRef === candidate.objectRef)
      .map((entry) => Date.parse(entry.at))
      .filter(Number.isFinite)
      .sort((a, b) => b - a)[0] ?? -Infinity;
  if (
    sent.some(
      (entry) =>
        entry.class === candidate.class &&
        entry.objectRef === candidate.objectRef &&
        entry.stateFingerprint === candidate.stateFingerprint &&
        Date.parse(entry.at) > recoveredAt,
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
  const attentionCandidates = attentionItems.flatMap((item) => tierItemCandidate(item, now));
  const drewActivity = snapshot.drewActivity ?? [];
  const candidates = attentionCandidates.filter((entry) => {
    const observedAt = Date.parse(entry.observedAt);
    return !(entry.actedRefs ?? []).some((ref) =>
      drewActivity.some((activity) => activity.objectRef === ref && Date.parse(activity.at) >= observedAt),
    );
  });

  for (const pool of snapshot.quota.pools) {
    // #7193 Codex: a pool whose quota read errored or went stale keeps its
    // last windows, but the measurement is marked unavailable — never
    // derive wall notifications from retained windows in that state.
    // A lapsed pool clears its windows at overlay, but still never burns.
    if (pool.state === "error" || pool.state === "stale" || pool.state === "lapsed") continue;
    const soleDoor = snapshot.soleDoorPoolIds?.has(pool.id) ?? false;
    const windows = pool.windows.filter((w) => w.limit !== null);
    const percentOf = (w: (typeof windows)[number]) => w.percent ?? (w.used / (w.limit || 1)) * 100;
    const wall = pool.state === "walled" || windows.some((w) => percentOf(w) >= 100);
    if (wall) {
      // #7193 Codex: the wall lifts when the EXHAUSTED windows reset (or at
      // the pool's authoritative wallLiftsAt) — an unrelated healthy
      // window's earlier reset must not suppress a genuine wall.
      const exhausted = windows.filter((w) => percentOf(w) >= 100);
      const wallLifts = pool.wallLiftsAt ? Date.parse(pool.wallLiftsAt) : NaN;
      const resetAt = Number.isFinite(wallLifts)
        ? wallLifts
        : (exhausted.length ? exhausted : windows).map((w) => Date.parse(w.resetsAt ?? "")).filter(Number.isFinite).sort((a, b) => a - b)[0];
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

/** Production sole-door classification (#7193 Codex): a pool is the only
 * route to its provider when no other pool in the view shares that
 * provider. server.ts supplies this on the live snapshot; tests may
 * still inject soleDoorPoolIds directly. */
export function deriveSoleDoorPoolIds(quota: QuotaView): Set<string> {
  const perProvider = new Map<string, number>();
  for (const pool of quota.pools) perProvider.set(pool.provider, (perProvider.get(pool.provider) ?? 0) + 1);
  return new Set(quota.pools.filter((pool) => perProvider.get(pool.provider) === 1).map((pool) => pool.id));
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
