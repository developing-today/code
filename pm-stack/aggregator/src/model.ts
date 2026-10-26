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
  dns: string;
  registry: string;
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

// ---------------------------------------------------------------------------
// Slice 3 (#906): pluggable pages, dashboard comments, the DNS register
// view, and the agent/session identity registry.
// ---------------------------------------------------------------------------

/** One row of the DNS/domain register (#902's register file, parsed). */
export interface DnsEntry {
  domain: string;
  registrar: string;
  dnsHost: string;
  /** Next renewal date, "YYYY-MM-DD". */
  renewal: string;
  /** "ON" | "OFF" | "unknown" — as the register states it. */
  autoRenew: string;
  /** List renewal price in USD, when the register names one. */
  priceUsd: number | null;
  priorYearCost: string;
  /** The register's "Where it's used" evidence cell. */
  used: string;
  /** The register's "Plan / status" cell. */
  plan: string;
  /** The register's "Review?" cell. */
  review: string;
  /** Derived from `plan`: in use, earmarked for a project, or unused. */
  status: "active" | "earmarked" | "unused";
}

/** The register as the collector embeds it in the fleet feed. */
export interface DnsSection {
  /** ISO timestamp of the collector run that parsed the register. */
  generatedAt: string;
  /** Where the register came from (file + pipeline owner). */
  source: string;
  /** Sum of list renewal prices across the register, USD/yr. */
  totalAnnualListUsd: number;
  entries: DnsEntry[];
}

/**
 * A dashboard comment (#906 §2). Distinct from the projector's `Comment`
 * (GitHub's own stream): these are written ON the dashboard, keyed by a
 * stable object ref — "gh:<owner/repo#N>" for GitHub-backed objects,
 * "orch:<tier item id>", "session:<sessionId>", a board item id
 * ("kaneo:…"/"gh:…"), or a roadmap subject ("grist:…") — so threads
 * survive feed republishes.
 */
export type DashCommentKind = "message" | "note" | "github";

/** Who wrote a dashboard comment. No anonymous comments exist. */
export interface DashAuthor {
  kind: "drew" | "muse" | "agent";
  /** Stable identity id ("drew", "muse", a registry/session id, …). */
  id: string;
  /** Human display name, rendered verbatim. */
  label: string;
  /** For agent authors: the session the comment came from, when known. */
  sessionId?: string;
}

export interface DashComment {
  id: string;
  /** The object ref the thread hangs on (see above). */
  objectRef: string;
  kind: DashCommentKind;
  author: DashAuthor;
  body: string;
  createdAt: string;
  /** kind "message": the orchestrator's triage state. */
  triageState?: "new" | "triaged";
  /** kind "github": the explicit target issue/PR the comment posts to. */
  githubRef?: RefKey;
  /** kind "github": pending until a fleet poster lands it, then posted. */
  githubState?: "pending" | "posted" | "failed";
  /** kind "github", once posted: the live comment URL. */
  githubUrl?: string;
}

/** Runtime families the session registry tracks. */
export type RegistrySource = SessionSource | "cron";

/**
 * One session in the identity registry (#906 §3): a stable id, a human
 * label, the agent identity it ran as, its parent (sub-sessions), its
 * lifecycle, its token totals where the source reports them, and the
 * task refs it is loosely linked to (from its PR/title/launch record).
 */
export interface SessionRecord {
  /** The session's own stable id (OpenCode ses_…, Jules id, launch id). */
  id: string;
  source: RegistrySource;
  /** Human display name (session title, or a launch-record label). */
  label: string;
  /** Agent identity: "opencode:main", "opencode:<subagent>", "claude-code", "jules", "cron:<job>", … */
  agentId: string;
  /** Display name for the agent identity. */
  agentLabel: string;
  /** Parent session id for sub-sessions, else null. */
  parentId: string | null;
  startedAt?: string;
  lastSeenAt?: string;
  /** Runtime-reported outcome or lifecycle label ("in-progress", …). */
  status: string;
  model?: string;
  title?: string;
  directory?: string;
  /** Cumulative tokens (input+output+reasoning), when reported. */
  tokensTotal?: number;
  /** Loose task linkage: GitHub refs this session is working. */
  taskRefs: RefKey[];
}
