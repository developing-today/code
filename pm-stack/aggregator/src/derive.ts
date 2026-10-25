// Pure derivations from the read model: the execution strip, the roadmap
// timeline lanes, and the unified comment stream. No I/O here.
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
const DATE = /^(\d{4})-(\d{2})-\d{2}/;

/**
 * Maps a Horizon cell to its timeline lane. Dates fold into their quarter;
 * "Ongoing", "Unscheduled" and anything else sort after every quarter.
 */
export function laneFor(horizon: string): Lane {
  const h = horizon.trim();
  const q = QUARTER.exec(h);
  if (q) return quarterLane(Number(q[2]), Number(q[1]));
  const d = DATE.exec(h);
  if (d) return quarterLane(Number(d[1]), Math.floor((Number(d[2]) - 1) / 3) + 1);
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

import type {
  AttentionTier,
  BoardItem,
  OrchestrationSummary,
  SessionSource,
  TieredEntry,
} from "./model.ts";

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
const DESTRUCTIVE_Q =
  /(\brm\b|delete|drop table|destroy|force[- ]?push|reset --hard|spend|purchase|\bpay\b|deploy)/i;
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

/**
 * Assign each classified entry its tier. Rule order matters — the first
 * match wins, and an orchestration escalation (#899) outranks every
 * local rule, carrying its reason verbatim.
 */
export function tierEntry(e: AttentionEntry, summary?: OrchestrationSummary | null): TieredEntry {
  const esc = summary?.escalations.find(
    (x) => (x.sessionId && x.sessionId === e.sessionId) || (x.title && x.title === e.title),
  );
  if (esc && e.state !== "in-motion") {
    return { ...e, tier: "needs-drew", drewReason: esc.reason };
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
    } else if (DECISION_Q.test(q)) {
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

export interface AttentionView {
  drew: TieredEntry[];
  orchestrator: TieredEntry[];
  /** Every current entry, tier-annotated, attention-ordered. */
  all: TieredEntry[];
  /** Finished/stale entries dropped before display. */
  expiredCount: number;
}

/** Classify, expire, and tier a raw session feed in one pass. */
export function buildAttention(
  feed: SessionFeedEntry[],
  summary?: OrchestrationSummary | null,
  now = Date.now(),
): AttentionView {
  const { current, expired } = partitionFresh(feed, now);
  const all = current
    .map((e) => tierEntry(classifyAttention(e, now), summary))
    .sort((a, b) => a.order - b.order || (b.lastActivityAt ?? "").localeCompare(a.lastActivityAt ?? ""));
  return {
    drew: all.filter((e) => e.tier === "needs-drew"),
    orchestrator: all.filter((e) => e.tier === "needs-orchestrator"),
    all,
    expiredCount: expired.length,
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
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];
const DAY_NAMES = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

export function ymdLabel(ymd: string): string {
  const d = new Date(`${ymd}T12:00:00Z`);
  return `${DAY_NAMES[d.getUTCDay()]} ${MONTH_NAMES[d.getUTCMonth()].slice(0, 3)} ${d.getUTCDate()}`;
}

export function monthLabel(ym: string): string {
  const [y, m] = ym.split("-").map(Number);
  return `${MONTH_NAMES[m - 1]} ${y}`;
}

// --- Roadmap grains (#898 §3) ----------------------------------------------

export type RoadmapGrain = "quarter" | "month" | "week" | "3d" | "today";

/** The row's calendar date when its Horizon is an ISO date, else null. */
export function rowDate(row: RoadmapRow): string | null {
  const m = YMD.exec(row.horizon.trim());
  return m ? `${m[1]}-${m[2]}-${m[3]}` : null;
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
    lanes: days.map((d) => ({ key: d, label: d === today ? `${ymdLabel(d)} · today` : ymdLabel(d), rows: lanes.get(d) ?? [] })),
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
  /** True when rendered from the #899 orchestration summary. */
  fromOrchestrator: boolean;
  generatedAt?: string;
}

const HORIZON_ORDER: Record<string, number> = { today: 0, tomorrow: 1, "3d": 2, week: 3, longer: 4 };
const PRIORITY_ORDER: Record<string, number> = { urgent: 0, high: 1, normal: 2, medium: 2, low: 3 };

export function deriveToday(
  board: BoardItem[],
  rows: RoadmapRow[],
  items: Map<RefKey, GhItem>,
  summary: OrchestrationSummary | null,
  now = new Date(),
): TodayView {
  if (summary) {
    return {
      fromOrchestrator: true,
      generatedAt: summary.generatedAt,
      objectives: [...summary.objectives]
        .sort((a, b) => (HORIZON_ORDER[a.horizon ?? "longer"] ?? 5) - (HORIZON_ORDER[b.horizon ?? "longer"] ?? 5))
        .map((o) => ({
          title: o.title,
          detail: [o.status, o.detail].filter(Boolean).join(" — ") || undefined,
          chip: `orchestrator${o.horizon ? ` · ${o.horizon}` : ""}`,
          ref: o.ref,
        })),
      momentum: summary.momentum.map((m) => ({
        project: m.project,
        state: m.state,
        accomplished: m.accomplished ?? [],
        blockedBy: m.blockedBy,
        detail: m.detail,
      })),
    };
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
  for (const item of board.filter((b) => b.status === "in-progress").sort(byPriority).slice(0, 5)) {
    objectives.push({ title: item.title, chip: `in progress · ${item.repo}` });
  }
  for (const item of board.filter((b) => b.status === "in-review").sort(byPriority).slice(0, 3)) {
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
    const blocked = all.filter(
      (i) => i.state === "open" && i.labels.some((l) => /blocked/i.test(l)),
    );
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
  return { fromOrchestrator: false, objectives, momentum };
}
