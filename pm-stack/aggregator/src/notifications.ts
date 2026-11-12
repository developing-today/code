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
import type { NeedsDrewQuestion } from "./needs-drew.ts";
import { decisionSetProgress, type DecisionSet } from "./decisions.ts";
import { SENTINEL_SERVICE_IDS, type StackHealthRecord } from "./stack-health.ts";

export const NOTIFICATION_CLASS_DEFAULTS: Record<string, boolean> = {
  incident: true,
  "needs-drew": true,
  "quota-wall": true,
  "quota-near-wall": true,
  "section-risk": true,
  "stack-health": true,
  "blocked-age": true,
};

/** Notification preference classes (#3318, slice (d) of the #1254
 * design): the classes a subscription can toggle on the preferences
 * page (/alerts/preferences). Ids are exactly the keys of
 * NOTIFICATION_CLASS_DEFAULTS — the stored subscription `classes`
 * map and the engine's classEnabled check read the same keys, so a
 * toggle on the page is the next engine decision. */
export interface NotificationClassMeta {
  id: string;
  label: string;
  blurb: string;
}

export const NOTIFICATION_CLASSES: NotificationClassMeta[] = [
  {
    id: "incident",
    label: "Incident alerts",
    blurb: "A watchdog reports something down. Always immediate — quiet hours never hold an incident.",
  },
  {
    id: "needs-drew",
    label: "Needs Drew",
    blurb: "An item enters the needs-Drew tier on the attention page.",
  },
  {
    id: "quota-wall",
    label: "Quota wall",
    blurb: "A pool hits its limit well before reset, or a sole-door pool walls at all.",
  },
  {
    id: "quota-near-wall",
    label: "Quota near wall",
    blurb: "A sole-door pool crosses 85% of a window.",
  },
  {
    id: "section-risk",
    label: "Section failure / severe risk",
    blurb: "A whole route section (workhorse, speed, specialist, summit, small) fails or is at severe risk. Shows the pools pending wall, which are walled, and the soonest resets.",
  },
  {
    id: "stack-health",
    label: "Stack health",
    blurb: "A monitored stack service starts failing its probe.",
  },
  {
    id: "blocked-age",
    label: "Blocked-age reminders",
    blurb: "A needs-Drew item stays unacted for 24/48 hours, then weekly (digest-only except the 48-hour breakthrough).",
  },
];

/** Classes a per-class test send accepts: every preference class,
 * plus "digest" — the digest is not a stored preference (it is the
 * delivery shape quiet/digest-only candidates take), but its landing
 * is part of the deep-link acceptance, so it gets a test too. */
export const NOTIFICATION_TEST_CLASSES: string[] = [...NOTIFICATION_CLASSES.map((c) => c.id), "digest"];

export function isNotificationTestClass(value: unknown): value is string {
  return typeof value === "string" && NOTIFICATION_TEST_CLASSES.includes(value);
}

/** The test item's stable object ref for a class test send. The
 * `test:` prefix keeps test traffic distinguishable in the sends
 * ledger from real candidates of the same class. */
export const classTestObjectRef = (cls: string): string => `test:${cls}`;

/** Deep-link landing for a per-class test notification (design §3,
 * #3318 acceptance). With a real candidate in hand, the landing is
 * the candidate's own URL — the same URL its dashboard card links
 * to. Without one, the test item lands on the view the engine itself
 * assigns that class: incident → its anchored Alerts card, quota
 * classes → /quota, stack-health → /health, and the attention
 * classes → /attention anchored to the test item (mirroring
 * tierItemCandidate's fallback for items with no session/GitHub
 * ref). Never the origin root; the only list-shaped landing is the
 * digest case, filtered to the digest's refs. */
export function notificationTestLanding(
  cls: string,
  origin: string,
  candidate?: Pick<NotificationCandidate, "url"> | null,
): string {
  if (candidate?.url) return /^https?:\/\//.test(candidate.url) ? candidate.url : `${origin}${candidate.url}`;
  switch (cls) {
    case "incident":
      return `${origin}/alerts#incident-test`;
    case "quota-wall":
    case "quota-near-wall":
    case "section-risk":
      return `${origin}/quota`;
    case "stack-health":
      return `${origin}/health`;
    case "digest":
      return `${origin}/attention?refs=${encodeURIComponent("test:needs-drew,test:blocked-age")}`;
    case "needs-drew":
      return `${origin}/attention#test-needs-drew`;
    case "blocked-age":
      return `${origin}/attention#test-blocked-age`;
    default:
      return `${origin}/alerts`;
  }
}

export const NOTIFICATION_COOLDOWN_MS = 6 * 60 * 60_000;
export const BLOCKED_AGE_REMINDER_MS = 24 * 60 * 60_000;
export const BLOCKED_AGE_BREAKTHROUGH_MS = 48 * 60 * 60_000;
export const QUOTA_WELL_BEFORE_RESET_MS = 24 * 60 * 60_000;

export interface NotificationCandidate {
  class: "needs-drew" | "blocked-age" | "quota-wall" | "quota-near-wall" | "section-risk" | "stack-health";
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
  /** #8568: a raised needs-Drew question rides its candidate so the
   * push payload can carry tappable answer actions. */
  needsDrew?: { ledgerId: string; question: string; options: Array<{ id: string; label: string }> };
  /** #8766: a decision set rides its candidate so the push payload can
   * point at the token-carrying decisions page (no tappable answers —
   * the page is the answer surface). */
  decisionSet?: { setId: string };
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
  /** #8882: route section -> quota pool ids serving it, joined from
   * the deployed OpenCode config's route leaf sets (server.ts via
   * aa.ts sectionPoolIdsFromConfig). Absent = no section derivation. */
  sectionPools?: Readonly<Record<string, readonly string[]>>;
  /** Drew-authored dashboard actions/comments, with their event times. */
  drewActivity?: Array<{ objectRef: string; at: string }>;
  /** #8568: open needs-Drew questions (unanswered, < 7 days old). */
  needsDrewRaised?: NeedsDrewQuestion[];
  /** #8766: open decision sets (at least one unresolved question). */
  decisionSets?: DecisionSet[];
  /** #8568: sentinel services currently down. */
  stackHealthDown?: StackHealthRecord[];
  /** #8568: full sentinel state map (up AND down) — decides when the
   * sentinel is authoritative over the health-derived candidate. */
  stackHealthStates?: Record<string, StackHealthRecord>;
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

/** #8882: the major route sections Drew named for the section-risk
 * class — exactly these, no scout. Keep in sync with
 * SECTION_RISK_SECTIONS in aa.ts (the config-side join). */
export const SECTION_RISK_SECTIONS = ["workhorse", "speed", "specialist", "summit", "small"] as const;
/** Pending wall = a measured window at or over this percent (the
 * existing near-wall threshold), on a pool that is not walled. */
export const SECTION_RISK_PENDING_PERCENT = 85;

const RESET_WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/** Compact UTC reset stamp for push bodies: "Fri 14:28Z". */
export function formatResetCompact(ms: number): string {
  const d = new Date(ms);
  const hh = String(d.getUTCHours()).padStart(2, "0");
  const mm = String(d.getUTCMinutes()).padStart(2, "0");
  return `${RESET_WEEKDAYS[d.getUTCDay()]} ${hh}:${mm}Z`;
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

  // #8882: section-risk — one candidate per major route section, and
  // only when the section as a whole has FAILED (every serving pool
  // walled) or is at SEVERE RISK (every non-walled serving pool is
  // pending wall). Membership comes from the snapshot (the aa.ts leaf
  // join over the deployed config); pools in error/stale/lapsed state
  // never count as serving, exactly as in the per-pool quota loop.
  for (const section of SECTION_RISK_SECTIONS) {
    const memberIds = snapshot.sectionPools?.[section];
    if (!memberIds || memberIds.length === 0) continue;
    const memberSet = new Set(memberIds);
    interface ServingPool {
      id: string;
      walled: boolean;
      wallResetAt: number | null;
      pendingPct: number | null;
      pendingResetAt: number | null;
    }
    const serving: ServingPool[] = [];
    for (const pool of snapshot.quota.pools) {
      if (!memberSet.has(pool.id)) continue;
      if (pool.state === "error" || pool.state === "stale" || pool.state === "lapsed") continue;
      const windows = pool.windows.filter((w) => w.limit !== null);
      const percentOf = (w: (typeof windows)[number]) => w.percent ?? (w.used / (w.limit || 1)) * 100;
      const walled = pool.state === "walled" || windows.some((w) => percentOf(w) >= 100);
      let wallResetAt: number | null = null;
      let pendingPct: number | null = null;
      let pendingResetAt: number | null = null;
      if (walled) {
        // Same wall-lift computation as the quota-wall candidate: the
        // pool's authoritative wallLiftsAt, else the soonest reset of
        // the exhausted windows.
        const lifts = pool.wallLiftsAt ? Date.parse(pool.wallLiftsAt) : NaN;
        if (Number.isFinite(lifts)) {
          wallResetAt = lifts;
        } else {
          const exhausted = windows.filter((w) => percentOf(w) >= 100);
          const resets = (exhausted.length ? exhausted : windows)
            .map((w) => Date.parse(w.resetsAt ?? ""))
            .filter(Number.isFinite)
            .sort((a, b) => a - b);
          wallResetAt = resets[0] ?? null;
        }
      } else {
        for (const w of windows) {
          const pct = percentOf(w);
          if (pct >= SECTION_RISK_PENDING_PERCENT && (pendingPct === null || pct > pendingPct)) {
            pendingPct = Math.floor(pct);
            const reset = Date.parse(w.resetsAt ?? "");
            pendingResetAt = Number.isFinite(reset) ? reset : null;
          }
        }
      }
      serving.push({ id: pool.id, walled, wallResetAt, pendingPct, pendingResetAt });
    }
    if (serving.length === 0) continue;
    const walledPools = serving.filter((entry) => entry.walled);
    const pendingPools = serving.filter((entry) => !entry.walled && entry.pendingPct !== null);
    const failed = walledPools.length === serving.length;
    const severeRisk = !failed && pendingPools.length > 0 && pendingPools.length === serving.length - walledPools.length;
    if (!failed && !severeRisk) continue;
    const resetSort = (a: number | null, b: number | null) => (a ?? Infinity) - (b ?? Infinity);
    const walledPart = walledPools
      .slice()
      .sort((a, b) => resetSort(a.wallResetAt, b.wallResetAt))
      .map((entry) => `${entry.id}${entry.wallResetAt !== null ? ` (resets ${formatResetCompact(entry.wallResetAt)})` : ""}`)
      .join(", ");
    const pendingPart = pendingPools
      .slice()
      .sort((a, b) => resetSort(a.pendingResetAt, b.pendingResetAt))
      .map(
        (entry) =>
          `${entry.id} ${entry.pendingPct}%${entry.pendingResetAt !== null ? ` (resets ${formatResetCompact(entry.pendingResetAt)})` : ""}`,
      )
      .join(", ");
    const body = [walledPart ? `walled: ${walledPart}` : "", pendingPart ? `pending: ${pendingPart}` : ""]
      .filter(Boolean)
      .join(" \u00b7 ");
    const soonestReset =
      [...walledPools.map((entry) => entry.wallResetAt), ...pendingPools.map((entry) => entry.pendingResetAt)]
        .filter((value): value is number => value !== null)
        .sort((a, b) => a - b)[0] ?? null;
    candidates.push(
      candidate({
        class: "section-risk",
        objectRef: `section:${section}`,
        title: failed ? `Section failed: ${section}` : `Section at risk: ${section}`,
        body,
        url: "/quota",
        observedAt: new Date(now).toISOString(),
        state: {
          state: failed ? "failed" : "severe-risk",
          walled: walledPools.map((entry) => entry.id).sort(),
          pending: pendingPools.map((entry) => [entry.id, entry.pendingPct]).sort(),
          soonestReset: soonestReset !== null ? new Date(soonestReset).toISOString() : null,
        },
        breakthrough: failed,
      }),
    );
  }

  for (const service of snapshot.health.services) {
    if (service.ok !== false) continue;
    // #8568: the sentinel is authoritative for its service set — when
    // it holds any record (up or down) for this service, its candidate
    // (from stackHealthDown below) replaces the health-derived one.
    if (
      (SENTINEL_SERVICE_IDS as readonly string[]).includes(service.def.id) &&
      snapshot.stackHealthStates?.[service.def.id]
    ) {
      continue;
    }
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

  // #8568: raised needs-Drew questions — a fleet worker is actively
  // waiting, so these break through like the 48h blocked-age case.
  for (const item of snapshot.needsDrewRaised ?? []) {
    candidates.push(
      candidate({
        class: "needs-drew",
        objectRef: `needs-drew:${item.ledgerId}`,
        title: item.question,
        body: item.context ?? "A fleet worker is blocked on your answer.",
        url: `/answer/${encodeURIComponent(item.ledgerId)}`,
        observedAt: item.raisedAt,
        breakthrough: true,
        actedRefs: [`needs-drew:${item.ledgerId}`],
        state: { raisedAt: item.raisedAt },
        needsDrew: { ledgerId: item.ledgerId, question: item.question, options: item.options },
      }),
    );
  }

  // #8766: open decision sets — ONE push per set, never one per item
  // or question. The fingerprint is the set's raise time alone, so
  // resolutions landing later do not re-push; a fully resolved set
  // stops candidating (listOpenDecisionSets already filtered it out).
  for (const set of snapshot.decisionSets ?? []) {
    const progress = decisionSetProgress(set);
    candidates.push(
      candidate({
        class: "needs-drew",
        objectRef: `decision-set:${set.setId}`,
        title: `${progress.openItems} decision${progress.openItems === 1 ? "" : "s"} waiting for you`,
        body: `${set.title} — ${progress.openQuestions} question${progress.openQuestions === 1 ? "" : "s"} open. Tap to open the decisions page.`,
        url: `/decisions/${encodeURIComponent(set.setId)}`,
        observedAt: set.createdAt,
        breakthrough: true,
        actedRefs: [`decision-set:${set.setId}`],
        state: { raisedAt: set.createdAt },
        decisionSet: { setId: set.setId },
      }),
    );
  }

  // #8568: sentinel-reported down services.
  for (const svc of snapshot.stackHealthDown ?? []) {
    candidates.push(
      candidate({
        class: "stack-health",
        objectRef: `service:${svc.serviceId}`,
        title: `Stack health: ${svc.name}`,
        body: svc.detail ?? "The monitored service is failing.",
        url: "/health",
        observedAt: svc.since,
        state: { state: "down", since: svc.since },
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
