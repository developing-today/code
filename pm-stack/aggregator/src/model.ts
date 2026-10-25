// The aggregator's read model. Everything the views render is one of
// these shapes; every data source (fixtures, live readers) produces them.
//
// The keys follow the design record's link-registry contract:
// repo-qualified issue/PR references per item, and a comment-activity
// projection keyed by them.

/** The four working repositories (design record, "Request and intent"). */
export const REPOS = [
  "developing-today/code",
  "developing-today/opencode",
  "developing-today/openchamber",
  "dezren39/nix",
] as const;

/** "owner/repo#123" — the registry's canonical key for an issue or PR. */
export type RefKey = string;

export interface GhRef {
  repo: string; // owner/repo
  number: number;
}

export type GhKind = "issue" | "pr";
export type GhState = "open" | "closed" | "merged";

/** Live state of one GitHub issue or PR, as the projector projects it. */
export interface GhItem {
  ref: RefKey;
  kind: GhKind;
  url: string;
  title: string;
  state: GhState;
  draft?: boolean;
  labels: string[];
  author: string;
  updatedAt: string;
  /** PRs only: the latest review verdict, if any. */
  reviewDecision?: "approved" | "changes_requested" | "review_required" | null;
}

export type CommentKind = "issue_comment" | "review" | "review_comment";

/** One entry of the comment projection, PR-interior comments included. */
export interface Comment {
  id: string;
  ref: RefKey;
  kind: CommentKind;
  author: string;
  body: string;
  createdAt: string;
  url: string;
  /** review_comment: the file and line the comment is anchored to. */
  path?: string;
  line?: number;
  /** review: approved | changes_requested | commented. */
  reviewState?: string;
  /** The projector keeps deleted comments as tombstones. */
  deleted?: boolean;
}

/** Why a reference is attached to an item (registry relation). */
export type Relation = "tracks" | "implements" | "relates";

export interface Link {
  ref: RefKey;
  relation: Relation;
}

/** A board column. Kaneo's own column ids are kept. */
export type BoardStatus = "in-progress" | "in-review" | "done" | string;

/** One card on the cross-repo WIP board. */
export interface BoardItem {
  /** The registry subject: "kaneo:<projectSlug>/<taskId>" or "gh:<ref>". */
  id: string;
  title: string;
  /** Repository the item belongs to (its Kaneo project's repo, or the ref's). */
  repo: string;
  source: "kaneo" | "github";
  status: BoardStatus;
  priority?: string;
  updatedAt?: string;
  /** Kaneo task link, when the item lives on a Kaneo board. */
  kaneoUrl?: string;
  /** Every linked issue/PR, from any of the four repos. */
  links: Link[];
  /** Pad notes linked to the item (planning layer), by title + URL. */
  notes?: { title: string; url: string }[];
}

/** One row of the Grist "Roadmap" table (doc "Program roadmap"). */
export interface RoadmapRow {
  id: number;
  /** The registry subject: "grist:<docId>/<tableId>/<rowId>". */
  subject: string;
  title: string;
  area: string;
  status: string;
  horizon: string;
  source: string;
  notes: string;
  links: Link[];
}

/** Where each facet's data came from, shown in the UI footer. */
export interface Provenance {
  board: string;
  roadmap: string;
  items: string;
  comments: string;
  quota: string;
  attention: string;
}

// ---------------------------------------------------------------------------
// Quota + throughput view (#851). Machine-first: the same shapes the API
// returns are what the GUI renders. No code from the CCC reference design
// (FSL-1.1-MIT) is vendored — this is the data model only.
// ---------------------------------------------------------------------------

export type QuotaState = "healthy" | "near" | "walled";

/** One usage window a provider exposes (5-hour, weekly, monthly, ...). */
export interface QuotaWindow {
  name: string;
  unit: "requests" | "tokens" | "usd" | "messages" | "other";
  limit: number | null;
  used: number;
  /** ISO timestamp when this window resets. */
  resetsAt: string | null;
  /** Percent used, 0–100+, computed when `limit` is known. */
  percent?: number;
}

/** One pool (plan + identity) with its windows and live burn. */
export interface QuotaPool {
  id: string;
  provider: string;
  plan: string;
  /** E.g. "Antigravity has TWO pools: Gemini models, and the Claude models it supports." */
  note?: string;
  state: QuotaState;
  /** Set when walled: the ISO time the wall lifts. */
  wallLiftsAt?: string;
  windows: QuotaWindow[];
  /** tokens/min over the recent window, when measurable. */
  burnTokensPerMin?: number;
  /** Sessions currently burning this pool, with cumulative tokens. */
  activeSessions?: { session: string; model: string; tokens: number }[];
}

/** Historical burn between resets, so headroom is projected, not observed. */
export interface BurnSegment {
  provider: string;
  pool: string;
  from: string;
  to: string;
  tokens: number;
}

export interface QuotaView {
  pools: QuotaPool[];
  burn: BurnSegment[];
  /** Guidance for choosing the next pool (burn order + reset rules). */
  choices: {
    /** Pools with headroom for a new session, best first. */
    hasHeadroom: string[];
    /** Pools about to reset with unused headroom (burn-it-before-reset). */
    burnBeforeReset: string[];
    /** Pools too close to their limit to start a second session. */
    doNotStart: string[];
  };
}

// ---------------------------------------------------------------------------
// "Needs you" attention view (#852). One classification per live session,
// machine-first. Classification rules are in derive.ts, not here.
// ---------------------------------------------------------------------------

export type AttentionState =
  | "in-motion"
  | "stopped-for-question"
  | "errored"
  | "needs-poke"
  | "stalled-on-cli"
  | "stuck-retrying";

/** The raw feed entry for one session, as the projector / watch cron emits. */
export interface SessionFeedEntry {
  sessionId: string;
  title?: string;
  directory?: string;
  branch?: string;
  model?: string;
  provider?: string;
  lastActivityAt: string | null;
  messageCount?: number;
  messageDelta?: number;
  activeToolCalls?: number;
  lastTurnError?: string | null;
  /** A question the session is waiting on, if any. */
  awaitingQuestion?: string | null;
  /** Longest running shell/CLI call, with age, when one is open. */
  longRunningCall?: { command?: string; startedAt: string } | null;
  /** Count of consecutive identical errors (retry-loop detection). */
  repeatedErrorCount?: number;
  pr?: string | null;
  /** Keeper/supervision annotation for this session, when one exists
   *  (jules-review-state.json supervision entries mentioning the id). */
  supervisionNote?: string;
}

/** One classified session, with the evidence for its state inline. */
export interface AttentionEntry extends SessionFeedEntry {
  state: AttentionState;
  order: number; // lower = more urgent
  evidence: string;
}

export function refKey(ref: GhRef): RefKey {
  return `${ref.repo}#${ref.number}`;
}

export function parseRef(key: RefKey): GhRef {
  const m = /^([\w.-]+\/[\w.-]+)#(\d+)$/.exec(key);
  if (!m) throw new Error(`not a repo-qualified ref: ${key}`);
  return { repo: m[1], number: Number(m[2]) };
}

const GH_URL = /https:\/\/github\.com\/([\w.-]+\/[\w.-]+)\/(?:issues|pull)\/(\d+)/g;

/** Every GitHub issue/PR URL in a piece of text, as ref keys (deduplicated). */
export function refsInText(text: string): RefKey[] {
  const out = new Set<RefKey>();
  for (const m of text.matchAll(GH_URL)) out.add(`${m[1]}#${m[2]}`);
  return [...out];
}

export function refUrl(key: RefKey, kind: GhKind = "issue"): string {
  const { repo, number } = parseRef(key);
  return `https://github.com/${repo}/${kind === "pr" ? "pull" : "issues"}/${number}`;
}

// ---------------------------------------------------------------------------
// Orchestration section (#899): the recurring orchestration job's output,
// carried in the fleet feed and served at /api/orchestration.json. The
// dashboard's today view (#898) consumes it: tiered needs-you items with
// per-item reasons, a flat dependency view, and per-goal measurements.
// The orchestrator republishes the whole section every run; an item
// absent from the newest section has expired out.
// ---------------------------------------------------------------------------

/** Assignment/measurement horizon buckets (the #899 priority ladder). */
export type Horizon = "today" | "tomorrow" | "next-3-days" | "longer" | "incoming";

export type TierName = "needs-drew" | "needs-orchestrator";

/** Where a queue item came from — same source labels as the session feed. */
export type OrchestrationSource =
  | "opencode-sandbox"
  | "opencode-laptop"
  | "jules"
  | "claude"
  | "backlog"
  | "tracked"
  | "github";

/** A prerequisite: "blocked by X; X is being worked by session S". */
export interface DependencyRef {
  /** Human label of the prerequisite task. */
  task: string;
  /** "owner/repo#123" when the prerequisite is a GitHub item. */
  ref?: RefKey;
  status: "in-progress" | "queued" | "stalled" | "done" | "unknown";
  /** Session working the blocker, when one is. */
  workerSessionId?: string;
  /** Human label for the worker, e.g. "opencode-laptop · Jules 4259…". */
  workerLabel?: string;
}

/** One classified queue item in a needs-* tier. */
export interface TierItem {
  /** sessionId | "owner/repo#123" | "jules:<id>" | tracked goal id. */
  id: string;
  kind: "session" | "decision" | "task" | "gap";
  title: string;
  source: OrchestrationSource;
  /** AttentionState for sessions; a free label otherwise. */
  state?: string;
  /** One line: why this item sits in this tier. */
  reason: string;
  /** ISO: first classified into this tier. */
  since: string;
  /** ISO: most recent orchestrator run that still observed the item. */
  lastSeenAt: string;
  ref?: RefKey;
  sessionId?: string;
  /** The pending question, trimmed, for question-stopped items. */
  question?: string;
  /** Task items: ladder bucket. */
  horizon?: Horizon;
  blockedBy?: DependencyRef[];
  /** The orchestrator's recorded resolution or handoff note. */
  recommendedAction?: string;
}

/** One task lined up under a goal, with its ladder bucket. */
export interface LinedUpTask {
  title: string;
  ref?: RefKey;
  status: "in-progress" | "queued" | "blocked" | "done-recently";
  horizon: Horizon;
  /** Why this bucket (due date, Drew's pick, NEXT marker, …). */
  horizonWhy: string;
  blockedBy?: DependencyRef[];
}

/** The orchestrator's measurement of one of Drew's top goals. */
export interface GoalMeasurement {
  /** Tracked/user goal id when known. */
  goalId: string | null;
  goal: string;
  momentum: "moving" | "stalled" | "blocked" | "no-work-lined-up";
  /** One line. */
  summary: string;
  linedUp: LinedUpTask[];
  /** What progressed since the previous orchestrator run, as evidence lines. */
  progressedSinceLastRun: string[];
  stalled: { title: string; since: string; blocker?: string }[];
  /** Findings, e.g. "goal has no tasks lined up". */
  gaps: string[];
}

/** The whole orchestration section, as embedded in the fleet feed. */
export interface OrchestrationSection {
  version: 1;
  /** ISO timestamp of the orchestrator run that produced this section. */
  generatedAt: string;
  /** "orch-<ISO>" — the producing run. */
  runId: string;
  /** Sources + run, human-readable (the section's own provenance). */
  provenance: string;
  tiers: { needsDrew: TierItem[]; needsOrchestrator: TierItem[] };
  /** Flat cross-item dependency view for one-place rendering. */
  dependencies: (DependencyRef & { taskRef?: RefKey })[];
  goals: GoalMeasurement[];
  /** What the producing run pushed back into motion. */
  actions: { at: string; kind: string; target: string; summary: string }[];
}
