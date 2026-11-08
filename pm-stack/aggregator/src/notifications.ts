import type { AttentionView } from "./derive.ts";
import type { HealthView } from "./health.ts";
import type { QuotaView, TierItem, TieredEntry } from "./model.ts";

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
    | "queue-cancelled";
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
  const observedAt = isTierItem ? item.since : item.tierSince ?? item.createdAt ?? item.lastActivityAt ?? new Date(now).toISOString();
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
  const attentionCandidates = attentionItems.flatMap((item) => tierItemCandidate(item, now));
  const drewActivity = snapshot.drewActivity ?? [];
  const candidates = attentionCandidates.filter((entry) => {
    const observedAt = Date.parse(entry.observedAt);
    return !(entry.actedRefs ?? []).some((ref) =>
      drewActivity.some((activity) => activity.objectRef === ref && Date.parse(activity.at) >= observedAt),
    );
  });

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
