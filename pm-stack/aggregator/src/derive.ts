// Pure derivations from the read model: the execution strip, the roadmap
// timeline lanes, and the unified comment stream. No I/O here.
import type { SectionDiagnosis } from "./feed.ts";
import type { Comment, GhItem, Link, RefKey, RoadmapRow } from "./model.ts";

export interface ExecutionStrip {
  /** Draft PRs, and open issues already dispatched or in progress. */
  implementing: GhItem[];
  /** Open, non-draft PRs. */
  review: GhItem[];
  /** Merged PRs. */
  merged: GhItem[];
  /** Open issues not yet being implemented. */
  queued: GhItem[];
  /** Closed issues and closed-unmerged PRs. */
  closed: GhItem[];
  /** Linked refs the source could not resolve. */
  unknown: RefKey[];
}

const IMPLEMENTING_LABELS = new Set(["status:in-progress", "in-progress", "ready-for-agent"]);

export function executionStrip(links: Link[], items: Map<RefKey, GhItem>): ExecutionStrip {
  const strip: ExecutionStrip = {
    implementing: [],
    review: [],
    merged: [],
    queued: [],
    closed: [],
    unknown: [],
  };
  const seen = new Set<RefKey>();
  for (const { ref } of links) {
    if (seen.has(ref)) continue;
    seen.add(ref);
    const item = items.get(ref);
    if (!item) {
      strip.unknown.push(ref);
      continue;
    }
    if (item.kind === "pr") {
      if (item.state === "merged") strip.merged.push(item);
      else if (item.state === "closed") strip.closed.push(item);
      else if (item.draft) strip.implementing.push(item);
      else strip.review.push(item);
    } else if (item.state !== "open") {
      strip.closed.push(item);
    } else if (item.labels.some((l) => IMPLEMENTING_LABELS.has(l.toLowerCase()))) {
      strip.implementing.push(item);
    } else {
      strip.queued.push(item);
    }
  }
  return strip;
}

/** A timeline column: a calendar quarter, or a non-dated horizon. */
export interface Lane {
  key: string;
  label: string;
  order: number;
}

const QUARTER = /^Q([1-4])\s+(\d{4})$/i;
const DATE = /^(\d{4})-(\d{2})-(\d{2})/;

const DAYS_IN_MONTH = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/**
 * True when year/month/day form a real calendar date. The DATE/YMD
 * regexes check digit shape only, so month 00/13, day 00, Feb 30 and
 * Apr 31 all pass them; this rejects those (a JS Date would silently
 * roll them into a neighbouring month instead of failing).
 */
export function isCalendarDate(year: number, month: number, day: number): boolean {
  if (month < 1 || month > 12 || day < 1) return false;
  const leap = (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
  const maxDay = month === 2 && leap ? 29 : DAYS_IN_MONTH[month - 1];
  return day <= maxDay;
}

/**
 * Maps a Horizon cell to its timeline lane. Dates fold into their quarter;
 * "Ongoing", "Unscheduled" and anything else sort after every quarter.
 */
export function laneFor(horizon: string): Lane {
  const h = horizon.trim();
  const q = QUARTER.exec(h);
  if (q) return quarterLane(Number(q[2]), Number(q[1]));
  const d = DATE.exec(h);
  if (d && isCalendarDate(Number(d[1]), Number(d[2]), Number(d[3])))
    return quarterLane(Number(d[1]), Math.floor((Number(d[2]) - 1) / 3) + 1);
  if (/^ongoing$/i.test(h)) return { key: "ongoing", label: "Ongoing", order: 1e6 };
  if (h === "" || /^unscheduled$/i.test(h)) {
    return { key: "unscheduled", label: "Unscheduled", order: 1e6 + 2 };
  }
  return { key: `other:${h}`, label: h, order: 1e6 + 1 };
}

function quarterLane(year: number, quarter: number): Lane {
  return { key: `${year}-Q${quarter}`, label: `Q${quarter} ${year}`, order: year * 4 + quarter };
}

export interface Timeline {
  lanes: Lane[];
  themes: string[];
  /** cells[theme][laneKey] = rows */
  cells: Map<string, Map<string, RoadmapRow[]>>;
}

/** Themes (Area) × lanes (quarters, then non-dated horizons). */
export function timeline(rows: RoadmapRow[]): Timeline {
  const lanes = new Map<string, Lane>();
  const cells = new Map<string, Map<string, RoadmapRow[]>>();
  for (const row of rows) {
    const lane = laneFor(row.horizon);
    lanes.set(lane.key, lane);
    const theme = row.area || "Unthemed";
    if (!cells.has(theme)) cells.set(theme, new Map());
    const byLane = cells.get(theme) as Map<string, RoadmapRow[]>;
    byLane.set(lane.key, [...(byLane.get(lane.key) ?? []), row]);
  }
  return {
    lanes: [...lanes.values()].sort((a, b) => a.order - b.order),
    themes: [...cells.keys()],
    cells,
  };
}

/** One activity stream for an item: every linked ref's comments, newest first. */
export function commentStream(links: Link[], comments: Comment[]): Comment[] {
  const wanted = new Set(links.map((l) => l.ref));
  return comments
    .filter((c) => wanted.has(c.ref))
    .sort((a, b) => b.createdAt.localeCompare(a.createdAt) || a.id.localeCompare(b.id));
}

/** Every distinct ref across a set of items. */
export function allRefs(itemsWithLinks: { links: Link[] }[]): RefKey[] {
  return [...new Set(itemsWithLinks.flatMap((i) => i.links.map((l) => l.ref)))];
}

// ---------------------------------------------------------------------------
// Quota view derivations (#851).
// ---------------------------------------------------------------------------

import type { QuotaPool, QuotaWindow } from "./model.ts";

/**
 * Percent used for a window. When the limit is unknown but the source
 * read carried an explicit percent (percent-only console/dashboard
 * reads), that percent is authoritative; otherwise an unknown limit
 * counts as fully used (a wall we cannot see past).
 */
export function windowPercent(w: QuotaWindow): number {
  if (w.limit == null || w.limit <= 0) return w.percent ?? 100;
  return (w.used / w.limit) * 100;
}

/** Near-limit threshold, in percent. */
export const NEAR_PCT = 85;

/** A pool is walled when any window is at or over its limit. */
export function poolState(p: QuotaPool): "healthy" | "near" | "walled" {
  for (const w of p.windows) if (windowPercent(w) >= 100) return "walled";
  for (const w of p.windows) if (windowPercent(w) >= NEAR_PCT) return "near";
  return "healthy";
}

// ---------------------------------------------------------------------------
// Attention view derivations (#852). The classification rules combine
// server signals (status, message growth, active tool calls) with the
// watch-cron heuristics already proven in the id-train watches.
// ---------------------------------------------------------------------------

import type { AttentionEntry, AttentionState, SessionFeedEntry } from "./model.ts";

const ORDER: Record<AttentionState, number> = {
  "stopped-for-question": 0,
  errored: 1,
  "needs-poke": 2,
  "stuck-retrying": 3,
  "stalled-on-cli": 4,
  "in-motion": 5,
};

/** A shell call older than this is "stalled on CLI". */
export const CLI_STALL_MS = 30 * 60_000;

/** Retry-loop detection: this many consecutive identical errors. */
export const RETRY_LOOP_THRESHOLD = 3;

/**
 * Classify one session feed entry. Ordering matters: the first rule that
 * matches wins, so a walled-spin with a question still surfaces the question.
 */
export function classifyAttention(e: SessionFeedEntry, now = Date.now()): AttentionEntry {
  let state: AttentionState = "in-motion";
  let evidence = "Messages/tool calls progressing.";

  if (e.awaitingQuestion) {
    state = "stopped-for-question";
    evidence = `Waiting on: ${e.awaitingQuestion}`;
  } else if (e.lastTurnError) {
    state = "errored";
    evidence = `Last turn error: ${e.lastTurnError}`;
  } else if ((e.repeatedErrorCount ?? 0) >= RETRY_LOOP_THRESHOLD) {
    state = "stuck-retrying";
    evidence = `${e.repeatedErrorCount} consecutive identical errors, no progress.`;
  } else if (e.longRunningCall && Date.parse(e.longRunningCall.startedAt) + CLI_STALL_MS < now) {
    state = "stalled-on-cli";
    evidence = `CLI running long: ${e.longRunningCall.command ?? "unknown"} (since ${e.longRunningCall.startedAt}).`;
  } else if (!e.lastActivityAt || e.activeToolCalls === 0) {
    state = "needs-poke";
    evidence = "Idle with no completion signal and no question.";
  }

  return { ...e, state, order: ORDER[state], evidence };
}

/** Sort a feed by attention (most urgent first). */
export function attentionSorted(feed: SessionFeedEntry[], now = Date.now()): AttentionEntry[] {
  return feed.map((e) => classifyAttention(e, now)).sort((a, b) => a.order - b.order);
}

// ---------------------------------------------------------------------------
// Slice 2 (#898): session sources, freshness expiry, NEEDS DREW / NEEDS
// ORCHESTRATOR tiering, session hierarchy, roadmap grains, today view.
// ---------------------------------------------------------------------------

import type { AttentionTier, BoardItem, OrchestrationSection, SessionSource, TierItem, TieredEntry } from "./model.ts";

export function sourceOf(e: SessionFeedEntry): SessionSource {
  if (e.source) return e.source;
  return "opencode-sandbox"; // fleet-feed entries predate the stamp
}

/** Human labels for the six machine states (#898 §5: comprehensible). */
export const STATE_LABELS: Record<AttentionState, string> = {
  "in-motion": "working now",
  "stopped-for-question": "waiting for an answer",
  errored: "errored",
  "needs-poke": "idle — needs a poke",
  "stalled-on-cli": "long-running command",
  "stuck-retrying": "stuck retrying",
};

/** A finished run (runtime reported an outcome) expires after this long. */
export const FINISHED_EXPIRY_MS = 2 * 3_600_000;
/** Nothing presents as current past this age. */
export const MAX_ENTRY_AGE_MS = 24 * 3_600_000;

export function isFinished(e: SessionFeedEntry): boolean {
  return typeof e.outcome === "string" && e.outcome !== "";
}

/**
 * Split a feed into current entries and expired ones (#898 §2/§5):
 * finished-run leftovers expire FINISHED_EXPIRY_MS after their last
 * activity; anything older than MAX_ENTRY_AGE_MS expires regardless.
 * Entries with no timestamp are kept (no evidence of staleness).
 */
export function partitionFresh(
  feed: SessionFeedEntry[],
  now = Date.now(),
): { current: SessionFeedEntry[]; expired: SessionFeedEntry[] } {
  const current: SessionFeedEntry[] = [];
  const expired: SessionFeedEntry[] = [];
  for (const e of feed) {
    const t = e.lastActivityAt ? Date.parse(e.lastActivityAt) : NaN;
    if (!Number.isFinite(t)) {
      current.push(e);
      continue;
    }
    const age = now - t;
    if (age > MAX_ENTRY_AGE_MS || (isFinished(e) && age > FINISHED_EXPIRY_MS)) expired.push(e);
    else current.push(e);
  }
  return { current, expired };
}

const DECISION_Q =
  /(should (i|we)\b|which (option|approach|repo|one|of|path)|approve|approval|choose|pick one|sign[- ]?off|go ahead\?)/i;
const DESTRUCTIVE_Q = /(\brm\b|delete|drop table|destroy|force[- ]?push|reset --hard|spend|purchase|\bpay\b|deploy)/i;
const CREDENTIAL_ERR =
  /(otp|one[- ]?time code|verification code|passkey|2fa|sign[- ]?in required|login required|authentication required)/i;
const ESCALATION_NOTE = /(escalat|needs drew|drew decision|decision needed)/i;

const ORCHESTRATOR_NOTES: Record<AttentionState, string> = {
  "in-motion": "",
  "stopped-for-question": "Orchestrator answers from recorded context, or pokes the session to continue.",
  errored: "Orchestrator triages the error, then retries or reroutes the session.",
  "needs-poke": "Orchestrator pokes it — or closes it if the work is actually done.",
  "stuck-retrying": "Orchestrator breaks the retry loop and reroutes the work.",
  "stalled-on-cli": "Orchestrator checks the long-running command and nudges the session.",
};

/** The session id a #899 tier item names, when it names one. */
export function tierItemSessionId(item: TierItem): string | null {
  if (item.sessionId) return item.sessionId;
  if (item.id.startsWith("jules:")) return item.id.slice("jules:".length);
  if (item.kind === "session") return item.id;
  return null;
}

export interface SectionMatch {
  item: TierItem;
  tier: "needs-drew" | "needs-orchestrator";
}

/**
 * Assign each classified entry its tier. Rule order matters — the first
 * match wins, and the orchestration job's own classification (#899)
 * outranks every local rule, carrying its reason verbatim: the job
 * reviews these queues so Drew doesn't have to.
 */
export function tierEntry(e: AttentionEntry, match?: SectionMatch | null): TieredEntry {
  if (match && e.state !== "in-motion") {
    if (match.tier === "needs-drew") return { ...e, tier: "needs-drew", drewReason: match.item.reason };
    const next = match.item.recommendedAction ? ` Next: ${match.item.recommendedAction}` : "";
    return { ...e, tier: "needs-orchestrator", orchestratorNote: `${match.item.reason}${next}` };
  }
  if (e.state !== "in-motion" && e.supervisionNote && ESCALATION_NOTE.test(e.supervisionNote)) {
    return { ...e, tier: "needs-drew", drewReason: `Keeper escalated this session — ${e.supervisionNote}` };
  }
  if (e.state !== "in-motion" && e.awaitingQuestion) {
    const q = e.awaitingQuestion;
    if (/^permission pending:/i.test(q)) {
      if (DESTRUCTIVE_Q.test(q)) {
        return {
          ...e,
          tier: "needs-drew",
          drewReason: `A session is asking permission for something destructive — ${q}`,
        };
      }
      // Routine tool permissions are the fleet's to review, not Drew's.
    } else if (DECISION_Q.test(q) && sourceOf(e) !== "jules") {
      // Jules questions are the keeper's to answer (it replies with the
      // standard proceed/feedback messages) — they never need Drew
      // unless the orchestration job escalates them explicitly.
      return { ...e, tier: "needs-drew", drewReason: `Waiting on your decision — ${q}` };
    }
  }
  if (e.state !== "in-motion" && e.lastTurnError && CREDENTIAL_ERR.test(e.lastTurnError)) {
    return {
      ...e,
      tier: "needs-drew",
      drewReason: `Blocked on a sign-in only you can complete — ${e.lastTurnError}`,
    };
  }
  if (e.state === "in-motion") return { ...e, tier: "in-motion" };
  return { ...e, tier: "needs-orchestrator", orchestratorNote: ORCHESTRATOR_NOTES[e.state] };
}

/**
 * Where the tiers came from, honestly (#1139): when the orchestration
 * section is absent, invalid, or stale, the tier counts are not the
 * orchestrator's published queues and the page must say so at the
 * point of display instead of rendering a confident zero. `present`
 * means a valid section was served (stale counts as present — its
 * numbers render, badged old).
 */
export interface TierSourceInfo {
  present: boolean;
  state: SectionDiagnosis["state"];
  runId?: string;
  generatedAt?: string;
  failedChecks?: string[];
}

export interface AttentionView {
  drew: TieredEntry[];
  orchestrator: TieredEntry[];
  /** The deduped #1148 watchdog items merged into orchestratorExtra,
   *  exposed separately so JSON surfaces can publish the derived
   *  queue items without re-deriving them. */
  watchdog: TierItem[];
  /** #899 tier items that are not live sessions (decisions, tasks, gaps). */
  drewExtra: TierItem[];
  orchestratorExtra: TierItem[];
  /** Every current entry, tier-annotated, attention-ordered. */
  all: TieredEntry[];
  /** Finished/stale entries dropped before display. */
  expiredCount: number;
  /** The tier source's diagnosis (#1139); drives the banner + counts. */
  tierSource: TierSourceInfo;
}

/** Classify, expire, and tier a raw session feed in one pass. */
export function buildAttention(
  feed: SessionFeedEntry[],
  section?: OrchestrationSection | null,
  now = Date.now(),
  diagnosis?: SectionDiagnosis,
  watchdogItems: TierItem[] = [],
): AttentionView {
  // The tier source's state (#1139): the caller's diagnosis when it
  // has one (the live source tracks it in Source.diagnostics()); else
  // derive the coarse form from the section itself — a served section
  // is present, a missing one is absent. "invalid" can only come from
  // a real diagnosis, since an invalid section never reaches here as
  // a section.
  const tierSource: TierSourceInfo = diagnosis
    ? {
        present: diagnosis.state === "live" || diagnosis.state === "stale",
        state: diagnosis.state,
        runId: diagnosis.runId,
        generatedAt: diagnosis.generatedAt,
        failedChecks: diagnosis.failedChecks,
      }
    : section
      ? { present: true, state: "live", runId: section.runId, generatedAt: section.generatedAt }
      : { present: false, state: "absent" };
  const { current, expired } = partitionFresh(feed, now);
  const bySession = new Map<string, SectionMatch>();
  if (section) {
    for (const item of section.tiers.needsDrew) {
      const sid = tierItemSessionId(item);
      if (sid && !bySession.has(sid)) bySession.set(sid, { item, tier: "needs-drew" });
    }
    for (const item of section.tiers.needsOrchestrator) {
      const sid = tierItemSessionId(item);
      if (sid && !bySession.has(sid)) bySession.set(sid, { item, tier: "needs-orchestrator" });
    }
  }
  const consumed = new Set<string>();
  const all = current
    .map((raw) => {
      const e = classifyAttention(raw, now);
      const match = bySession.get(e.sessionId) ?? null;
      if (match) consumed.add(match.item.id);
      return tierEntry(e, match);
    })
    .sort((a, b) => a.order - b.order || (b.lastActivityAt ?? "").localeCompare(a.lastActivityAt ?? ""));
  // Section items that name a session in the feed are consumed by it
  // (tiered above, or in motion again); items naming expired sessions
  // expire with them. The rest are non-session queue items — decisions,
  // tasks, gaps — and render as cards in their tier.
  const knownIds = new Set([...current, ...expired].map((e) => e.sessionId));
  const extras = (items: TierItem[] | undefined): TierItem[] =>
    (items ?? []).filter((item) => {
      if (consumed.has(item.id)) return false;
      const sid = tierItemSessionId(item);
      return !(sid && knownIds.has(sid));
    });
  // #1148: watchdog items are aggregator-derived NEEDS ORCHESTRATOR
  // cards, merged alongside the orchestrator-published extras. They
  // are never session items, so they skip the extras() session
  // filtering; dedupe is by stable watchdog id, and a published item
  // with the same id (there cannot be one today) would win.
  const publishedExtra = extras(section?.tiers.needsOrchestrator);
  const publishedIds = new Set(publishedExtra.map((i) => i.id));
  const seenWatchdog = new Set<string>();
  const watchdogExtra = watchdogItems.filter((i) => {
    if (publishedIds.has(i.id) || seenWatchdog.has(i.id)) return false;
    seenWatchdog.add(i.id);
    return true;
  });
  return {
    drew: all.filter((e) => e.tier === "needs-drew"),
    orchestrator: all.filter((e) => e.tier === "needs-orchestrator"),
    watchdog: watchdogExtra,
    drewExtra: extras(section?.tiers.needsDrew),
    orchestratorExtra: [...publishedExtra, ...watchdogExtra],
    all,
    expiredCount: expired.length,
    tierSource,
  };
}

/**
 * Parent/child structure over a set of entries: an entry whose parentId
 * names another entry in the set is its child; anything else is a root
 * (a child whose parent is absent renders as a root with its chip).
 */
export function sessionChildren(entries: TieredEntry[]): Map<string, TieredEntry[]> {
  const ids = new Set(entries.map((e) => e.sessionId));
  const children = new Map<string, TieredEntry[]>();
  for (const e of entries) {
    if (e.parentId && e.parentId !== e.sessionId && ids.has(e.parentId)) {
      children.set(e.parentId, [...(children.get(e.parentId) ?? []), e]);
    }
  }
  return children;
}

export function sessionRoots(entries: TieredEntry[]): TieredEntry[] {
  const ids = new Set(entries.map((e) => e.sessionId));
  return entries.filter((e) => !(e.parentId && e.parentId !== e.sessionId && ids.has(e.parentId)));
}

// --- Calendar helpers (America/Chicago — Drew's timezone) ---------------

const YMD = /^(\d{4})-(\d{2})-(\d{2})/;

/** "2026-10-25" in America/Chicago for an instant. */
export function chicagoYmd(now = new Date()): string {
  return new Intl.DateTimeFormat("en-CA", {
    timeZone: "America/Chicago",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).format(now);
}

export function addDaysYmd(ymd: string, n: number): string {
  const d = new Date(`${ymd}T12:00:00Z`);
  d.setUTCDate(d.getUTCDate() + n);
  return d.toISOString().slice(0, 10);
}

/** Monday (Chicago) of the week containing `ymd`. */
export function weekStartYmd(ymd: string): string {
  const dow = new Date(`${ymd}T12:00:00Z`).getUTCDay(); // 0=Sun
  return addDaysYmd(ymd, -((dow + 6) % 7));
}

const MONTH_NAMES = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];
const DAY_NAMES = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

export function ymdLabel(ymd: string): string {
  const m = YMD.exec(ymd.trim());
  // Defensive: an impossible date is echoed verbatim instead of
  // throwing on MONTH_NAMES[NaN] or rolling over into the next month.
  if (!m || !isCalendarDate(Number(m[1]), Number(m[2]), Number(m[3]))) return ymd;
  const d = new Date(`${ymd}T12:00:00Z`);
  return `${DAY_NAMES[d.getUTCDay()]} ${MONTH_NAMES[d.getUTCMonth()].slice(0, 3)} ${d.getUTCDate()}`;
}

export function monthLabel(ym: string): string {
  const [y, m] = ym.split("-").map(Number);
  // Defensive: an out-of-range month is echoed verbatim instead of
  // rendering the literal "undefined" as a lane label.
  if (!Number.isInteger(m) || m < 1 || m > 12) return ym;
  return `${MONTH_NAMES[m - 1]} ${y}`;
}

// --- Roadmap grains (#898 §3) ----------------------------------------------

export type RoadmapGrain = "quarter" | "month" | "week" | "3d" | "today";

/** The row's calendar date when its Horizon is an ISO date, else null. */
export function rowDate(row: RoadmapRow): string | null {
  const m = YMD.exec(row.horizon.trim());
  if (!m || !isCalendarDate(Number(m[1]), Number(m[2]), Number(m[3]))) return null;
  return `${m[1]}-${m[2]}-${m[3]}`;
}

export interface DayLane {
  key: string;
  label: string;
  rows: RoadmapRow[];
}

export interface DayGrain {
  lanes: DayLane[];
  /** Dated rows outside the window. */
  outside: number;
  /** Rows with no calendar date (quarter grain, ongoing, unscheduled). */
  coarse: RoadmapRow[];
}

/** Day-grained roadmap: week (Mon–Sun), next 3 days, or today. */
export function roadmapDayGrain(rows: RoadmapRow[], grain: "week" | "3d" | "today", now = new Date()): DayGrain {
  const today = chicagoYmd(now);
  const days: string[] =
    grain === "today"
      ? [today]
      : grain === "3d"
        ? [today, addDaysYmd(today, 1), addDaysYmd(today, 2)]
        : Array.from({ length: 7 }, (_, i) => addDaysYmd(weekStartYmd(today), i));
  const inWindow = new Set(days);
  const lanes = new Map(days.map((d) => [d, [] as RoadmapRow[]]));
  let outside = 0;
  const coarse: RoadmapRow[] = [];
  for (const row of rows) {
    const d = rowDate(row);
    if (!d) {
      coarse.push(row);
    } else if (inWindow.has(d)) {
      lanes.get(d)?.push(row);
    } else {
      outside++;
    }
  }
  return {
    lanes: days.map((d) => ({
      key: d,
      label: d === today ? `${ymdLabel(d)} · today` : ymdLabel(d),
      rows: lanes.get(d) ?? [],
    })),
    outside,
    coarse,
  };
}

/** Month-grained timeline: dated rows by month; quarter rows keep their grain. */
export function monthTimeline(rows: RoadmapRow[]): Timeline {
  const lanes = new Map<string, Lane>();
  const cells = new Map<string, Map<string, RoadmapRow[]>>();
  const put = (row: RoadmapRow, lane: Lane) => {
    lanes.set(lane.key, lane);
    const theme = row.area || "Unthemed";
    if (!cells.has(theme)) cells.set(theme, new Map());
    const byLane = cells.get(theme) as Map<string, RoadmapRow[]>;
    byLane.set(lane.key, [...(byLane.get(lane.key) ?? []), row]);
  };
  for (const row of rows) {
    const d = rowDate(row);
    if (d) {
      const ym = d.slice(0, 7);
      const [y, m] = ym.split("-").map(Number);
      put(row, { key: ym, label: monthLabel(ym), order: y * 12 + m });
    } else {
      put(row, laneFor(row.horizon)); // quarter / ongoing / unscheduled lanes
    }
  }
  return {
    lanes: [...lanes.values()].sort((a, b) => a.order - b.order),
    themes: [...cells.keys()],
    cells,
  };
}

// --- Today view (#898 §4) ----------------------------------------------------

export interface TodayObjective {
  title: string;
  detail?: string;
  /** Where the objective came from, e.g. "orchestrator · today". */
  chip: string;
  ref?: RefKey;
}

export interface ProjectMomentum {
  project: string;
  state: "moving" | "stalled" | "blocked";
  accomplished: string[];
  blockedBy?: string;
  detail?: string;
}

export interface TodayView {
  objectives: TodayObjective[];
  momentum: ProjectMomentum[];
  /** True when rendered from the #899 orchestration section. */
  fromOrchestrator: boolean;
  generatedAt?: string;
  /** What the orchestrator pushed back into motion (section path only). */
  actions?: { at: string; kind: string; target: string; summary: string }[];
  /**
   * The orchestration section's diagnosis (#1139): the derived view's
   * banner names the true state (absent vs invalid vs stale) instead
   * of always claiming the job "has not published yet".
   */
  sectionDiagnosis?: SectionDiagnosis;
}

const PRIORITY_ORDER: Record<string, number> = { urgent: 0, high: 1, normal: 2, medium: 2, low: 3 };

/**
 * The orchestrator sometimes publishes `progressedSinceLastRun` as a
 * single string rather than a list (its declared shape). Normalize at
 * this boundary so every downstream consumer can trust `string[]`.
 */
export function asStringList(v: unknown): string[] {
  if (Array.isArray(v)) return v.map((x) => String(x));
  if (typeof v === "string" && v) return [v];
  return [];
}

/** Map the #899 section's goal measurements into the today view. */
export function todayFromSection(section: OrchestrationSection): TodayView {
  const blockedText = (deps?: { task: string; status: string; workerLabel?: string }[]): string | undefined =>
    deps?.length
      ? deps.map((d) => `${d.task} (${d.status}${d.workerLabel ? `, ${d.workerLabel}` : ""})`).join("; ")
      : undefined;
  const objectives: TodayObjective[] = [];
  for (const g of section.goals) {
    for (const task of g.linedUp.filter((t) => t.horizon === "today")) {
      objectives.push({
        title: task.title,
        detail:
          [task.status, task.horizonWhy, blockedText(task.blockedBy) && `blocked by ${blockedText(task.blockedBy)}`]
            .filter(Boolean)
            .join(" — ") || undefined,
        chip: `orchestrator · today · ${g.goal}`,
        ref: task.ref,
      });
    }
  }
  if (objectives.length === 0) {
    for (const g of section.goals) {
      objectives.push({ title: g.goal, detail: g.summary, chip: "orchestrator · goal" });
    }
  }
  const momentum: ProjectMomentum[] = section.goals.map((g) => {
    const blockers = [
      ...g.stalled.filter((s) => s.blocker).map((s) => `${s.title}: ${s.blocker}`),
      ...g.linedUp.flatMap((t) => (t.blockedBy?.length ? [`${t.title} — blocked by ${blockedText(t.blockedBy)}`] : [])),
    ];
    return {
      project: g.goal,
      state: g.momentum === "moving" ? "moving" : g.momentum === "blocked" ? "blocked" : "stalled",
      accomplished: asStringList(g.progressedSinceLastRun),
      blockedBy: blockers.join("; ") || undefined,
      detail: [g.summary, ...g.gaps].filter(Boolean).join(" "),
    };
  });
  return {
    fromOrchestrator: true,
    generatedAt: section.generatedAt,
    objectives,
    momentum,
    actions: section.actions.slice(-5).reverse(),
  };
}

export function deriveToday(
  board: BoardItem[],
  rows: RoadmapRow[],
  items: Map<RefKey, GhItem>,
  section: OrchestrationSection | null,
  now = new Date(),
  diagnosis?: SectionDiagnosis,
): TodayView {
  if (section) {
    const view = todayFromSection(section);
    view.sectionDiagnosis = diagnosis ?? {
      state: "live",
      runId: section.runId,
      generatedAt: section.generatedAt,
    };
    return view;
  }

  // Derived until #899 lands: today's dated roadmap rows first, then the
  // board's in-progress work by priority, then what's in review.
  const today = chicagoYmd(now);
  const objectives: TodayObjective[] = [];
  for (const row of rows) {
    if (rowDate(row) === today && !/^done$/i.test(row.status)) {
      objectives.push({ title: row.title, chip: "roadmap · today", detail: `${row.area} · ${row.status}` });
    }
  }
  const byPriority = (a: BoardItem, b: BoardItem) =>
    (PRIORITY_ORDER[(a.priority ?? "normal").toLowerCase()] ?? 2) -
      (PRIORITY_ORDER[(b.priority ?? "normal").toLowerCase()] ?? 2) ||
    (b.updatedAt ?? "").localeCompare(a.updatedAt ?? "");
  for (const item of board
    .filter((b) => b.status === "in-progress")
    .sort(byPriority)
    .slice(0, 5)) {
    objectives.push({ title: item.title, chip: `in progress · ${item.repo}` });
  }
  for (const item of board
    .filter((b) => b.status === "in-review")
    .sort(byPriority)
    .slice(0, 3)) {
    objectives.push({ title: item.title, chip: `in review · ${item.repo}` });
  }

  const weekAgo = now.getTime() - 7 * 86_400_000;
  const twoDaysAgo = now.getTime() - 2 * 86_400_000;
  const momentum: ProjectMomentum[] = [];
  const repos = [...new Set(board.map((b) => b.repo))];
  for (const repo of repos) {
    const repoBoard = board.filter((b) => b.repo === repo);
    const linked = new Map<RefKey, GhItem>();
    for (const b of repoBoard) {
      for (const l of b.links) {
        const it = items.get(l.ref);
        if (it) linked.set(it.ref, it);
      }
    }
    const all = [...linked.values()];
    const accomplished = all
      .filter((i) => (i.state === "merged" || i.state === "closed") && Date.parse(i.updatedAt) >= weekAgo)
      .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
      .slice(0, 3)
      .map((i) => `${i.ref} — ${i.title}`);
    const blocked = all.filter((i) => i.state === "open" && i.labels.some((l) => /blocked/i.test(l)));
    const recent = all.some((i) => Date.parse(i.updatedAt) >= twoDaysAgo);
    const inProgress = repoBoard.some((b) => b.status === "in-progress" || b.status === "in-review");
    momentum.push({
      project: repo,
      state: blocked.length > 0 ? "blocked" : accomplished.length > 0 || (inProgress && recent) ? "moving" : "stalled",
      accomplished,
      blockedBy: blocked.length ? blocked.map((i) => `${i.ref} — ${i.title}`).join("; ") : undefined,
      detail: inProgress ? undefined : "nothing in progress on the board",
    });
  }
  return { fromOrchestrator: false, objectives, momentum, sectionDiagnosis: diagnosis ?? { state: "absent" } };
}

// ---------------------------------------------------------------------------
// Usage & billing hub derivations (hub design §1.3, §4). Bubble rules are
// GOAL.md-fixed: any metric over 80% of its limit, or projected unbilled
// PAYG over $5 (unbilled accrual only — Q3 default), surfaces worst-first.
// ---------------------------------------------------------------------------

import { countedEstimate, type AttentionFlag, type QuotaView, type SnapshotRecord, type UsageView } from "./model.ts";

export const BUBBLE_PCT = 80;
export const BUBBLE_USD = 5;

/**
 * Every tripped bubble rule across both families. `usage.payg` is the
 * aggregation point for PAYG estimates, so a service whose estimate is
 * already a PAYG line is not double-counted (service ids listed in
 * `paygLineServices`).
 */
export function buildAttentionFlags(
  quota: QuotaView,
  usage: Pick<UsageView, "services" | "payg">,
  paygLineServices: ReadonlySet<string> = new Set(["cloudflare"]),
): AttentionFlag[] {
  const flags: AttentionFlag[] = [];
  for (const pool of quota.pools) {
    for (const w of pool.windows) {
      const pct = windowPercent(w);
      if (pct > BUBBLE_PCT) {
        flags.push({
          kind: "over-limit",
          label: `${pool.provider} · ${pool.plan} — ${w.name}`,
          detail: `${Math.round(pct)}% of limit used${w.resetsAt ? ` · resets ${w.resetsAt.slice(0, 10)}` : ""}`,
          href: "/quota",
          severity: pct,
        });
      }
    }
  }
  for (const svc of usage.services) {
    for (const m of svc.metrics) {
      if (m.percent != null && m.percent > BUBBLE_PCT) {
        flags.push({
          kind: "over-limit",
          label: `${svc.name} — ${m.label}`,
          detail: `${m.lowerBound ? "at least " : ""}${Math.round(m.percent)}% of limit used`,
          href: `/quota/service/${svc.id}`,
          severity: m.percent,
        });
      }
    }
    if (svc.paygEstimate && !paygLineServices.has(svc.id)) {
      const counted = countedEstimate(svc.paygEstimate);
      if (counted > BUBBLE_USD) {
        flags.push({
          kind: "payg-unbilled",
          label: `${svc.name} — PAYG estimate`,
          detail: `counted $${counted.toFixed(2)} unbilled (1.2 × high $${svc.paygEstimate.high.toFixed(2)})`,
          href: `/quota/service/${svc.id}`,
          severity: counted,
        });
      }
    }
  }
  for (const line of usage.payg) {
    if (line.unbilledUsd != null && line.unbilledUsd > BUBBLE_USD) {
      flags.push({
        kind: "payg-unbilled",
        label: line.label,
        detail: `$${line.unbilledUsd.toFixed(2)} projected unbilled${line.estimate ? ` (estimate $${line.estimate.low.toFixed(2)}–$${line.estimate.high.toFixed(2)})` : ""}`,
        href: "/quota",
        severity: line.unbilledUsd,
      });
    }
  }
  // Worst first: over-limit by percent, then PAYG by dollars — the two
  // severities are different units, so kinds stay grouped, worst kind
  // (over-limit) first per the design's "worst first" reading order.
  return flags.sort((a, b) => (a.kind === b.kind ? b.severity - a.severity : a.kind === "over-limit" ? -1 : 1));
}

export interface Freshness {
  stale: boolean;
  /** Whole days since the freshest read; null when never measured. */
  ageDays: number | null;
}

/**
 * How far a measuredAt may sit in the future before it stops being
 * believable (#1568). Date-only granularity and timezone noise can put a
 * stamp a few hours — at most a day — ahead; beyond that the stamp is
 * skewed or wrong, and its age is honestly unknown.
 */
export const FRESHNESS_FUTURE_TOLERANCE_MS = 86_400_000; // 1 day

/** A card is stale when its freshest read is older than its cadence (§1.1). */
export function freshness(measuredAt: string | null, cadenceDays: number, now = new Date()): Freshness {
  if (!measuredAt) return { stale: true, ageDays: null };
  // Date-only stamps age by calendar day (a read dated yesterday is
  // 1 day old all day today); datetimes age by elapsed whole days.
  // The calendar is Chicago's, the timezone every other calendar
  // helper here uses (#1629): a stamp is a Chicago date, so ageing it
  // against the UTC day called a read dated today "1d ago" -- and
  // stale -- from 19:00 Chicago onward, when UTC had rolled over and
  // Chicago had not.
  if (measuredAt.length <= 10) {
    const t = Date.parse(`${measuredAt}T00:00:00Z`);
    if (Number.isNaN(t)) return { stale: true, ageDays: null };
    const startOfToday = Date.parse(`${chicagoYmd(now)}T00:00:00Z`);
    // A date-only stamp more than the tolerance past today's Chicago
    // midnight is a future stamp, not a fresh one (#1568): without
    // this bound the clamp below would pin it "today" until its
    // date arrives.
    if (t - startOfToday > FRESHNESS_FUTURE_TOLERANCE_MS) return { stale: true, ageDays: null };
    const ageDays = Math.max(0, Math.round((startOfToday - t) / 86_400_000));
    return { stale: ageDays > cadenceDays, ageDays };
  }
  const t = Date.parse(measuredAt);
  if (Number.isNaN(t)) return { stale: true, ageDays: null };
  if (t - now.getTime() > FRESHNESS_FUTURE_TOLERANCE_MS) return { stale: true, ageDays: null };
  const ageDays = Math.max(0, Math.floor((now.getTime() - t) / 86_400_000));
  return { stale: ageDays > cadenceDays, ageDays };
}

export interface PaceProjection {
  metricId: string;
  /** Points used in the fit (after the last reset boundary). */
  points: number;
  /** Percent-points burned per day. */
  slopePerDay: number;
  /** ISO when the metric reaches 100% at the current slope, if it does. */
  projectedExhaustionAt: string | null;
  /** Projected percent at the window's recorded reset, when known. */
  projectedPercentAtReset: number | null;
}

const RESET_DROP_PCT = 30;

/** Index just after the last reset boundary (a resetsAt passing, or a big drop). */
function windowStartIndex(recs: SnapshotRecord[]): number {
  let start = 0;
  for (let i = 1; i < recs.length; i++) {
    const prev = recs[i - 1];
    const cur = recs[i];
    const resetPassed = prev.resetsAt != null && Date.parse(prev.resetsAt) <= Date.parse(cur.ts);
    const dropped =
      typeof prev.value === "number" && typeof cur.value === "number" && prev.value - cur.value >= RESET_DROP_PCT;
    if (resetPassed || dropped) start = i;
  }
  return start;
}

/**
 * Linear pace projection for one pct metric from its snapshot history
 * (§4: the computation quota-view.md does by hand for Claude). Null
 * when history cannot support a projection — never invented.
 */
export function projectPace(records: SnapshotRecord[], metricId: string): PaceProjection | null {
  const recs = records
    .filter((r) => r.metricId === metricId && r.unit === "pct" && typeof r.value === "number")
    .sort((a, b) => a.ts.localeCompare(b.ts));
  const win = recs.slice(windowStartIndex(recs));
  if (win.length < 2) return null;
  const first = win[0];
  const last = win[win.length - 1];
  const days = (Date.parse(last.ts) - Date.parse(first.ts)) / 86_400_000;
  if (days < 0.5) return null;
  const slope = ((last.value as number) - (first.value as number)) / days;
  if (slope <= 0.05) return null;
  const lastVal = last.value as number;
  // Out-of-domain pct (#2346): a negative percent-of-limit can never
  // support a projection, and an unbounded extrapolation can land
  // outside Date's range, where toISOString() throws RangeError. The
  // contract is null when history cannot support a projection --
  // slope and points are still reported.
  let projectedExhaustionAt: string | null = null;
  if (lastVal >= 100) {
    projectedExhaustionAt = last.ts;
  } else if (lastVal >= 0) {
    const candidate = new Date(Date.parse(last.ts) + ((100 - lastVal) / slope) * 86_400_000);
    if (Number.isFinite(candidate.getTime())) projectedExhaustionAt = candidate.toISOString();
  }
  let projectedPercentAtReset: number | null = null;
  if (last.resetsAt) {
    const toReset = (Date.parse(last.resetsAt) - Date.parse(last.ts)) / 86_400_000;
    if (toReset > 0) projectedPercentAtReset = Math.round((lastVal + slope * toReset) * 10) / 10;
  }
  return {
    metricId,
    points: win.length,
    slopePerDay: Math.round(slope * 100) / 100,
    projectedExhaustionAt,
    projectedPercentAtReset,
  };
}

/** Reset points in a metric's history, for chart markers (§4, §1.5). */
export function resetMarkers(records: SnapshotRecord[], metricId: string): { ts: string; resetsAt: string }[] {
  const recs = records.filter((r) => r.metricId === metricId).sort((a, b) => a.ts.localeCompare(b.ts));
  const out: { ts: string; resetsAt: string }[] = [];
  for (let i = 1; i < recs.length; i++) {
    const prev = recs[i - 1];
    const cur = recs[i];
    if (prev.resetsAt && Date.parse(prev.resetsAt) <= Date.parse(cur.ts)) {
      out.push({ ts: prev.resetsAt, resetsAt: prev.resetsAt });
    } else if (
      typeof prev.value === "number" &&
      typeof cur.value === "number" &&
      prev.value - cur.value >= RESET_DROP_PCT
    ) {
      out.push({ ts: cur.ts, resetsAt: cur.ts });
    }
  }
  return out;
}
