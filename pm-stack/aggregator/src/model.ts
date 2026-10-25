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
  orchestration: string;
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

/** Which runtime a session belongs to (slice 2, #898 §5). */
export type SessionSource = "opencode-sandbox" | "opencode-laptop" | "jules" | "claude-code";

export const SESSION_SOURCE_LABELS: Record<SessionSource, string> = {
  "opencode-sandbox": "OpenCode · sandbox",
  "opencode-laptop": "OpenCode · laptop",
  jules: "Jules",
  "claude-code": "Muse",
};

/** The raw feed entry for one session, as the projector / watch cron emits. */
export interface SessionFeedEntry {
  sessionId: string;
  /** Owning runtime; feed entries default to the sandbox server. */
  source?: SessionSource;
  /** Parent session id for sub-sessions (OpenCode `parentID`), else null/absent. */
  parentId?: string | null;
  /** Subagent name when the session is a spawned sub-session (e.g. "explore"). */
  agent?: string;
  /** Run outcome when the runtime reports one (succeeded/failed/interrupted). */
  outcome?: string | null;
  createdAt?: string;
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

/**
 * Slice 2 tiers (#898 §2): who has to act on a session. NEEDS DREW is the
 * default view and only holds items with an explainable `drewReason`;
 * everything else the fleet can work itself is NEEDS ORCHESTRATOR.
 */
export type AttentionTier = "needs-drew" | "needs-orchestrator" | "in-motion";

export interface TieredEntry extends AttentionEntry {
  tier: AttentionTier;
  /** Why this needs Drew — present iff tier is "needs-drew". */
  drewReason?: string;
  /** What the fleet will do with it — present iff tier is "needs-orchestrator". */
  orchestratorNote?: string;
}

/** One session source's wiring status, for the coverage note (#898 §5). */
export interface SourceCoverage {
  source: SessionSource;
  label: string;
  wired: boolean;
  detail: string;
}

// ---------------------------------------------------------------------------
// Orchestration summary (#899 -> today view, #898 §4). Written by the
// orchestration job on the sandbox as orchestration-state.json and
// embedded into the fleet feed by the collector. The aggregator renders
// it verbatim; until it exists the today view derives and says so.
// ---------------------------------------------------------------------------

export interface OrchestrationObjective {
  title: string;
  horizon?: "today" | "tomorrow" | "3d" | "week" | "longer";
  status?: string;
  detail?: string;
  /** Repo-qualified ref the objective tracks, when it has one. */
  ref?: RefKey;
}

export interface OrchestrationMomentum {
  project: string;
  state: "moving" | "stalled" | "blocked";
  accomplished?: string[];
  blockedBy?: string;
  detail?: string;
}

export interface OrchestrationEscalation {
  sessionId?: string;
  title?: string;
  reason: string;
  at: string;
}

export interface OrchestrationSummary {
  generatedAt: string;
  objectives: OrchestrationObjective[];
  momentum: OrchestrationMomentum[];
  escalations: OrchestrationEscalation[];
  notes?: string[];
}

/** Keeper run totals for Jules, parsed from jules-review-state.json. */
export interface JulesSummary {
  /** When the keeper last ran (its `last_run`). */
  at: string | null;
  /** Sessions the keeper currently tracks in its `items` map. */
  tracked: number;
  /** Totals from the keeper's freshest run note, when it parses. */
  total?: number;
  completed?: number;
  failed?: number;
  inProgress?: number;
  awaitingFeedback?: number;
  awaitingPlan?: number;
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
