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
/** Per-item resolution source (#1141): which tier of Source.items()
 *  produced a GhItem. "pool" and "projector" are live data; "fixture"
 *  is invented fallback state and must never render as if it were live. */
export type ItemResolution = "pool" | "projector" | "fixture";

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
  /** Project-pool items only (#910 stage 5): the org Project's fields. */
  projectStatus?: string | null;
  projectArea?: string | null;
  projectHorizon?: string | null;
  projectKind?: string | null;
  /** How this item was resolved (#1141). Absent = the source did not
   *  say (e.g. whole-source fixtures mode, whose provenance already
   *  reports fixtures for every facet). */
  resolution?: ItemResolution;
}

/**
 * One item of the org GitHub Project pool (#910 stage 5): a Project
 * item plus its linked issue/PR, as read by the fleet-feed collector
 * via GraphQL Projects v2 and carried in the feed's `projects`
 * section. This is the live resolution source for GhItems — the
 * Projects-scope token can read the Project (whose items carry their
 * linked issue's state) but not arbitrary issues.
 */
export interface ProjectPoolItem {
  /** Project item node id (PVTI_…). */
  id: string;
  /** Project item database id, used in the item URL's itemId param. */
  databaseId?: number;
  /** URL of the item in the Project (board + item pane). */
  url: string;
  /** URL of the Project itself. */
  projectUrl: string;
  /** "owner/repo#123" — same canonical key as GhItem.ref. */
  ref: RefKey;
  repo: string;
  number: number;
  /** URL of the linked issue/PR. */
  issueUrl: string;
  title: string;
  /** Linked issue/PR state (PRs may be "merged"). */
  state: GhState;
  contentKind: GhKind;
  draft?: boolean;
  labels?: string[];
  author?: string;
  updatedAt?: string;
  /** Project single-select fields; null when unset on the item. */
  status: string | null;
  area: string | null;
  horizon: string | null;
  /** The Project's "Kind" field (Task | Epic) — not the GhItem kind. */
  kind: string | null;
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
  source: "kaneo" | "github" | "orchestrator";
  /** Orchestrator in-flight entries only: the linedUp task's own status
   *  ("in-progress" | "queued" | "blocked"); the board status of such an
   *  entry is the synthetic lane key "in-flight" instead. */
  orchStatus?: string;
  /** Orchestrator in-flight entries only: the goal it is lined up under. */
  goal?: string;
  /** Orchestrator in-flight entries only: flattened blocked-by labels. */
  blockedBy?: string[];
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
  /** Grist UI link for this row's document (#1136), when known. */
  gristUrl?: string;
}

/** Where each facet's data came from, shown in the UI footer. */
export interface Provenance {
  board: string;
  roadmap: string;
  items: string;
  comments: string;
  quota: string;
  usage: string;
  attention: string;
  orchestration: string;
  dns: string;
  registry: string;
  aa: string;
}

/**
 * One facet's freshness, structured (#1138): the human string above,
 * plus the machine state the freshness chips render. `count` always
 * ships with its noun (`countLabel`) so a chip can never show a bare
 * number that contradicts a page header counting the same thing.
 */
export interface ProvenanceFacet {
  /** The provenance string, unchanged — also the chip's tooltip. */
  text: string;
  state: "live" | "fixtures" | "stale" | "absent" | "invalid";
  ageMs?: number;
  count?: number;
  countLabel?: string;
}

/** Per-facet freshness detail, keyed exactly like Provenance. */
export type ProvenanceDetail = Record<keyof Provenance, ProvenanceFacet>;

// ---------------------------------------------------------------------------
// Quota + throughput view (#851). Machine-first: the same shapes the API
// returns are what the GUI renders. No code from the CCC reference design
// (FSL-1.1-MIT) is vendored — this is the data model only.
// ---------------------------------------------------------------------------

/** "error"/"stale" are gatherer-overlay states (#1151): the gatherer
 * could not read the pool, or its read is past the freshness bound.
 * They are never rendered as healthy, and never as zero usage. */
export type QuotaState = "healthy" | "near" | "walled" | "error" | "stale";

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
  /** Gatherer freshness for this pool (#1151), set when
   * quota-current.json carried an entry for it. */
  freshness?: PoolFreshness;
}

/** Per-pool freshness from the quota gatherer (#1151). */
export interface PoolFreshness {
  measuredAt: string;
  source: string;
  state: "ok" | "stale" | "error";
  /** Why the gatherer could not read the pool, when state is "error". */
  error?: string;
}

/** One provider's freshness in the gatherer summary. */
export interface GathererProviderFreshness {
  providerId: string;
  serviceId: string;
  credentialLabel?: string;
  state: "ok" | "stale" | "error";
  measuredAt: string;
  source: string;
  error?: string;
}

/** The gatherer's presence in the quota payload (#1151): tick time,
 * health, and every provider it read — including providers with no
 * pool on the page (OpenCode Go workspaces, Antigravity models). */
export interface GathererSummary {
  generatedAt: string;
  source: string;
  tickAt: string | null;
  ok: boolean;
  providers: GathererProviderFreshness[];
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
  /** Gatherer overlay provenance (#1151); absent when no
   * quota-current.json was read. Additive for existing consumers. */
  gatherer?: GathererSummary;
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
  | "github"
  /** Aggregator-synthesized watchdog items (#1148): derived live from
   *  health/probe diagnoses, never published by the orchestrator. */
  | "watchdog";

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
  /** #1290 extension fields — all optional, validated per item fail-soft
   *  in the live source (src/tier-items.ts); the section gate in feed.ts
   *  never learns them, so a malformed extension drops, never the item. */
  whyUser?: WhyUserClass;
  /** 1–3 sentence plain-language version of the item. */
  summary?: string;
  /** What happens if this sits unanswered. When the publisher omits it,
   *  the live source computes the fallback from `since`. */
  ifIgnored?: string;
  options?: TierItemOption[];
  questions?: TierItemQuestion[];
  delegation?: TierItemDelegation;
}

/** Why an item needs the user, per the #1290 contract. Naming is
 *  role-based (2026-10-27 decision): no new Drew-named identifiers — the
 *  legacy `needsDrew` tier array name is the published shape and stays. */
export type WhyUserClass = "decision" | "missing-info" | "approval" | "access" | "policy";

/** A publisher-offered answer option on a needs-user item (#1290).
 *  Picking one posts the `answer` verb with the option's id. */
export interface TierItemOption {
  id: string;
  label: string;
  description?: string;
  /** Predicted effect of picking this option, shown under its button. */
  effect?: string;
}

export type TierItemQuestionType = "single" | "multi" | "text" | "yesno";

/** One survey question on a needs-user item (#1290). `covers` names the
 *  decision the answer settles; it is mandatory, so a survey can never
 *  be decoration. */
export interface TierItemQuestion {
  id: string;
  prompt: string;
  type: TierItemQuestionType;
  options?: { id: string; label: string }[];
  /** The sanitizer defaults an absent flag to true. */
  required: boolean;
  covers: string;
}

export type DelegationMode = "user" | "first-pass" | "auto";
export type DelegationStatus = "idle" | "working" | "proposal-ready" | "unable" | "done";

/** A drafted, NOT-yet-applied first-pass proposal (#1290/#1293). Present
 *  exactly when the delegation status is `proposal-ready`. Proposal
 *  identity is (item id, delegation runId, delegation updatedAt). */
export interface DelegationProposal {
  summary: string;
  detail?: string;
  proposedChanges?: string[];
  at: string;
}

/** The structured inability record — the ONLY way a delegated item comes
 *  back to the user (#1292). Present exactly when status is `unable`. */
export interface DelegationUnable {
  reason: string;
  whatItTried: string[];
  whatItNeeds: string;
  at: string;
}

/** Delegation state for a needs-user item (#1290), validated as a unit
 *  by the per-item sanitizer: any inconsistency drops the whole field,
 *  never the item. */
export interface TierItemDelegation {
  mode: DelegationMode;
  status: DelegationStatus;
  updatedAt: string;
  runId?: string;
  proposal?: DelegationProposal;
  unable?: DelegationUnable;
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
/**
 * The orchestration section's own provenance (#1110): the sources the
 * producing run read, as the orchestration job publishes them. The
 * object form is the contract — the publisher has always emitted it,
 * and it is strictly richer than a joined string. The aggregator's
 * feed boundary also accepts the legacy plain-string form older runs
 * published and normalizes it to this shape (see feed.ts), so every
 * consumer sees exactly this type.
 */
export interface OrchestrationProvenance {
  /** One human-readable note per source the run read. */
  sources: string[];
  /** The producing run, when the publisher records it here too. */
  runId?: string;
}

export interface OrchestrationSection {
  version: 1;
  /** ISO timestamp of the orchestrator run that produced this section. */
  generatedAt: string;
  /** "orch-<ISO>" — the producing run. */
  runId: string;
  /** What the producing run read (the section's own provenance). */
  provenance: OrchestrationProvenance;
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

// ---------------------------------------------------------------------------
// AA cost explorer: the Artificial Analysis workbook data (Supabase
// `ainventory`, latest import batch) as the collector embeds it in the
// fleet feed. The figures are the workbook snapshot's — AA Intelligence
// Index v4.3.2, Test_Runs snapshots of Sep–Oct 2026 — not a live AA pull;
// the section's provenance says exactly which batch and files they are.
// ---------------------------------------------------------------------------

/** One workbook source file behind the AA section (freshness evidence). */
export interface AaSourceInfo {
  name: string;
  kind: string;
  /** When the ainventory import read the file. */
  importedAt: string | null;
  /** The file's own modification time in Drive. */
  driveModifiedAt: string | null;
}

/** One row of ainventory.v_rankings (the explorer's headline table). */
export interface AaRanking {
  rowIndex: number;
  testRunKey: string;
  modelVariantKey: string;
  /** Display family name, e.g. "GPT-6.1 Sol". */
  family: string;
  creator: string;
  effort: string | null;
  benchmarkVersionKey: string | null;
  rawScore: number | null;
  adjustedScore: number | null;
  costTask: number | null;
  timeTaskSec: number | null;
  speedTps: number | null;
  qualStatus: string | null;
  availability: string | null;
  eligible: boolean;
  intelEffPerUsd: number | null;
  usdPerUsableUnit: number | null;
  pointsPerDollar: number | null;
  pointsPerSecond: number | null;
  balancedValue: number | null;
  smartValue: number | null;
  speedValue: number | null;
  rankBalanced: number | null;
  rankSmart: number | null;
  rankSpeed: number | null;
  rankIntelligence: number | null;
  rankLowCost: number | null;
}

/** One row of ainventory.v_threshold_metrics: is this config "usable"
 * (clears the surplus bar) at a given target intelligence level? */
export interface AaThreshold {
  testRunKey: string;
  target: number;
  usable: boolean;
  levelledScore: number | null;
  costTask: number | null;
  /** Minutes per task at this config. */
  timeMin: number | null;
  usdPerUsablePoint: number | null;
}

/** One AA Intelligence Index row of ainventory.benchmark_results. */
export interface AaConfig {
  modelName: string;
  benchmark: string;
  /** The workbook's effort label, verbatim (e.g. "Adaptive Reasoning,
   * High Effort, Default Fallback") — never shortened or invented. */
  effort: string | null;
  intelligence: number | null;
  costPerTaskUsd: number | null;
  speedTps: number | null;
}

/** One Test_Runs row, mapped by the sheet's own header row: per-config
 * timing. Null timing = the workbook never measured it — the page says
 * "not measured", it does not estimate. */
export interface AaTestRun {
  testRunKey: string;
  modelVariantKey: string;
  snapshotDate: string | null;
  intelligence: number | null;
  costPerTaskUsd: number | null;
  outputSpeedTps: number | null;
  ttftSeconds: number | null;
  firstAnswerSeconds: number | null;
  e2e500Seconds: number | null;
  timePerTaskSeconds: number | null;
}

/**
 * One Coding Agent Index row of ainventory.benchmark_results (the
 * workbook's Coding_Agent_Results import): a model + effort + agent
 * harness observation, kept strictly separate from the AA Intelligence
 * Index — no score conversion between the indexes (the ChatGPT
 * explorers' cross-benchmark rule). Component scores are the published
 * fractions (0–1) of DeepSWE v1.1 / Terminal-Bench 4.0 / SWE-Atlas-QnA;
 * the page renders them as percents.
 */
export interface AaCaiRow {
  /** Model name as reported, effort included, e.g. "GPT-6.1 Sol (xhigh)". */
  modelName: string;
  /** Family display name: modelName with the trailing "(effort)" removed. */
  family: string;
  /** Creator display name, mapped from the observation key's creator slug. */
  creator: string;
  /** Effort label as reported ("not stated" stays "not stated"). */
  effort: string | null;
  /** Benchmark version, e.g. "v1.5" (one historical "v1.4" row rides along). */
  version: string | null;
  /** Coding Agent Index score (0–100 scale of that index). */
  score: number | null;
  /** Published cost per coding task, USD; null = not published/comparable. */
  cost: number | null;
  /** Average agent wall time per task, seconds. */
  wallSec: number | null;
  /** Total token usage per task. */
  tokensPerTask: number | null;
  /** Agent harness the observation ran under, e.g. "Codex". */
  harness: string | null;
  deepSwe: number | null;
  terminalBench: number | null;
  sweAtlas: number | null;
  snapshotDate: string | null;
  sourceUrl: string | null;
}

/**
 * One current token price from the ainventory event log
 * (v_current_prices — the #1248 append-only price events, resolved to
 * current): USD per 1M tokens for one model id (models.dev-style
 * "provider/model"), one context band, one rate window. windowKind is
 * "flat" unless the provider runs a time-of-day schedule.
 */
export interface AaPrice {
  /** Models.dev-style id the price is recorded under. */
  modelName: string;
  inputPer1M: number | null;
  outputPer1M: number | null;
  cacheReadPer1M: number | null;
  cacheWritePer1M: number | null;
  contextMinTokens: number | null;
  contextMaxTokens: number | null;
  windowKind: string | null;
}

/**
 * Pool/catalog coverage, computed by the collector from the repo's
 * .opencode/opencode.jsonc (provider catalog + model_routes leaves,
 * nested routes expanded) against the AA-measured model names.
 * Best-effort: name matching is normalization + a small alias map in
 * src/aa.ts, so a miss means "not matched", never "does not exist".
 */
export interface AaCoverage {
  /** AA-measured models with no seat in any route pool. */
  measuredNotPooled: string[];
  /** The subset of those that a provider catalog does carry — measured,
   * reachable, but unrouted (the actionable list). */
  catalogNotPooledMeasured: string[];
  /** Measured models no provider catalog carries under a matched name. */
  measuredNotInCatalog: string[];
  /** Catalog model ids (provider/model) with no AA measurement and no
   * pool seat — e.g. commandcode/reflection/beam:free. Display names
   * are catalog ids, not AA names, so they can be found in the config. */
  catalogNotPooledUnmeasured: string[];
}

/**
 * One explorer data point: a rankings row joined with its Test_Runs
 * timing row (same testRunKey) and enriched with the derived values
 * the Cost Explorer's charts and perspectives plot — per-dollar and
 * per-second intelligence, Pareto-frontier flags for each chart's axis
 * pair, and the 3-axis dominators ("beaten by") list in the reference
 * design's sense. Built by aaPoints() in src/aa.ts; null always means
 * "not measured", never an estimate.
 */
export interface AaPoint {
  key: string;
  family: string;
  creator: string;
  effort: string | null;
  benchmark: string | null;
  score: number | null;
  cost: number | null;
  time: number | null;
  tps: number | null;
  ttft: number | null;
  pointsPerDollar: number | null;
  pointsPerSecond: number | null;
  /** score / cost, computed when the workbook's own column is absent. */
  scorePerDollar: number | null;
  /** score / time, computed when the workbook's own column is absent. */
  scorePerSecond: number | null;
  eligible: boolean;
  qualStatus: string | null;
  /** Agent harness (CAI points only; null on AA points). */
  harness: string | null;
  /** Total tokens per task (CAI points only; the workbook's AA rows
   * do not carry a per-task token count). */
  tokensPerTask: number | null;
  /** CAI component scores as published fractions (CAI points only). */
  components: { deepSwe: number | null; terminalBench: number | null; sweAtlas: number | null } | null;
  /** On the cost×score Pareto frontier (cheaper-or-equal, smarter-or-equal unbeaten). */
  frontierCost: boolean;
  /** On the time×score Pareto frontier. */
  frontierTime: boolean;
  /** On the cost×time Pareto frontier (lower-left unbeaten). */
  frontierCostTime: boolean;
  /** "Family effort" labels beating this point on score + cost + time at once. */
  beatenBy: string[];
}

/**
 * One model in the fleet's own catalog (the Model Explorer tab): a
 * provider/model entry from .opencode/opencode.jsonc, its pool seats
 * (route ids whose expanded leaves include it), the plan the provider
 * door belongs to (collector-carried registry facts — see
 * AA_PLAN_FACTS in src/aa.ts), and its best AA measurement when the
 * workbook measured a model of the same normalized name. Assembled by
 * aaCatalogFromConfig() in src/aa.ts.
 */
export interface AaCatalogEntry {
  /** Full catalog id, "provider/model…" (model part may contain /). */
  id: string;
  provider: string;
  /** The provider's display name in the config, else its id. */
  providerName: string;
  model: string;
  /** The model's display name in the config, else the model id part. */
  name: string;
  contextLimit: number | null;
  outputLimit: number | null;
  /** Effort variant ids the config defines for this model, ladder order. */
  variants: string[];
  /** Route ids seating this model (nested routes expanded). */
  routes: string[];
  /** The provider sits in the config's disabled_providers — catalogued but not callable. */
  disabled: boolean;
  /** Plan label from the collector's registry facts, when one is recorded. */
  plan: string | null;
  /** Effective value multiplier of the plan (list-rate dollars per $1), when recorded. */
  multiplier: number | null;
  /** Short plan note (caps, expiries, caveats), when recorded. */
  planNote: string | null;
  /**
   * The model's published OpenCode Go monthly cap in USD, where Go lists
   * one (docs reading, AA value report 2026-10-20) — per-model caps, not
   * a shared pool; the fleet's older workspaces may meter pooled.
   */
  goCapUsd: number | null;
  /** Best AA intelligence measured for this model name, when measured. */
  aaScore: number | null;
  /** $/task of that best-scoring AA measurement. */
  aaCostPerTask: number | null;
  /** Fastest measured time/task among the family's ranked rows. */
  aaTimePerTask: number | null;
  /**
   * Current token rates (USD per 1M) from the ainventory price event
   * log, when a price is recorded for this model — the Model Explorer's
   * task-projection calculator prices from these, and only these.
   * `bands` are the recorded context bands (the projection picks the
   * band the task's per-turn input falls in, like Site 2's long-tier
   * rule); `windowed` marks a provider with a time-of-day schedule
   * (the projection's DeepSeek peak/off-peak toggle reads it).
   */
  rate?: {
    bands: {
      minTokens: number | null;
      maxTokens: number | null;
      input: number | null;
      output: number | null;
      cacheRead: number | null;
      cacheWrite: number | null;
      /** Rate window of the price row: "flat", or a time-of-day window
       * ("peak" / "offpeak") for scheduled providers (DeepSeek). */
      window: string | null;
    }[];
    windowed: boolean;
  } | null;
}

/** The AA explorer section as the collector embeds it in the fleet feed. */
export interface AaSection {
  status: "ok";
  /** ISO timestamp of the collector run that built the section. */
  generatedAt: string;
  /** ainventory import batch (v_latest_workbook_batch) the rows came from. */
  batch: string;
  sources: AaSourceInfo[];
  rankings: AaRanking[];
  thresholds: AaThreshold[];
  configs: AaConfig[];
  testRuns: AaTestRun[];
  /** Absent when the collector could not read .opencode/opencode.jsonc. */
  coverage?: AaCoverage;
  /** The fleet catalog for the Model Explorer tab, from the same
   * config read as coverage; absent on the same condition. */
  catalog?: AaCatalogEntry[];
  /** Coding Agent Index rows of the same batch (separate benchmark
   * family — the explorer's "Coding agents" perspective); absent when
   * the collector's payload predates them. */
  cai?: AaCaiRow[];
  /** Current token prices from the ainventory price event log
   * (v_current_prices); absent on the same condition as cai. */
  prices?: AaPrice[];
}

/** The section's degraded form: the collector could not reach Supabase
 * (or the payload failed its check) — the page renders the reason
 * instead of stale numbers, and the feed never fails over it. */
export interface AaAbsentSection {
  status: "absent";
  reason: string;
  generatedAt: string;
}

export type AaFeedSection = AaSection | AaAbsentSection;

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

// ---------------------------------------------------------------------------
// Usage & billing hub (design 2026-10-25, hub-design §1–§4). Family 2
// (SaaS services), the PAYG dollars view, charges evidence, the bubble
// flags, and the append-only snapshot store's record shape. The hub
// evolves the quota page in place: QuotaView above stays untouched for
// its existing consumers; UsageView rides beside it via Source.usage().
// ---------------------------------------------------------------------------

/** How a number was obtained — manual/console reads are first-class. */
export type UsageSourceClass = "api" | "console" | "manual" | "estimate" | "invoice";

export type UsageUnit =
  | "pct"
  | "usd"
  | "count"
  | "gb"
  | "tokens"
  | "minutes"
  | "hours"
  | "ms"
  | "rows"
  | "ops"
  | "neurons"
  | "other";

/** One measured (or allowance-only) metric on a SaaS service card. */
export interface UsageMetric {
  id: string;
  label: string;
  /** null = an allowance is recorded but usage was not measured. */
  used: number | null;
  limit: number | null;
  unit: UsageUnit;
  /** 0–100+, present when used and limit are both known (or unit is pct). */
  percent?: number;
  /** `used` is a lower bound (e.g. capped API result), so derived
   * percentages must read "at least N%". */
  lowerBound?: boolean;
  /** In the §2 highlight set (front page); false = detail view only. */
  highlight: boolean;
  note?: string;
}

/**
 * A Decision-004 PAYG estimate: low/high as recorded. The counted
 * figure (1.2 × high) is derived at render time, never stored.
 */
export interface PaygEstimate {
  low: number;
  high: number;
  /** What the low/high describe, e.g. "per month" or "Oct 24–25 accrual". */
  period: string;
  note?: string;
}

/** The counted figure for an estimate: 1.2 × high (Decision 004). */
export function countedEstimate(e: PaygEstimate): number {
  return Math.round(e.high * 1.2 * 100) / 100;
}

export type SaasGroup = "database" | "object-storage" | "compute" | "dev-platform" | "domains" | "account-tooling";

export const SAAS_GROUP_LABELS: Record<SaasGroup, string> = {
  database: "Database",
  "object-storage": "Object storage",
  compute: "Compute / platform",
  "dev-platform": "Dev platform",
  domains: "Domains",
  "account-tooling": "Account tooling / flat",
};

/** One dated source read, preserved verbatim-ish for the detail view. */
export interface DatedRead {
  /** ISO date (or datetime) of the read. */
  at: string;
  text: string;
}

/** One family-2 service: a card on the hub, a page at /quota/service/<id>. */
export interface SaasService {
  id: string;
  name: string;
  group: SaasGroup;
  /** Plan + price as recorded, e.g. "Workers Paid $5/mo + R2 Paid $0 base". */
  plan: string;
  status: "active" | "candidate" | "no-account" | "unknown";
  metrics: UsageMetric[];
  /** The recorded PAYG estimate, or null when none exists. */
  paygEstimate: PaygEstimate | null;
  /** Why there is no estimate: "not applicable — …" / "not estimable from local records". */
  paygNote?: string;
  /** USD invoiced/charged this period, when a record states it. */
  billedUsd?: number | null;
  /** ISO date/datetime of the freshest dated read behind this card. */
  measuredAt: string | null;
  sourceClass: UsageSourceClass;
  /** Where the card's facts came from, human-readable. */
  sourceNote: string;
  /** Renewal/reset as recorded (may be prose, e.g. "cycle ends 2026-10-31"). */
  nextDate?: string | null;
  /** Expected re-measure cadence in days; older reads render stale. */
  cadenceDays: number;
  /** Dated disagreements between sources, shown not resolved (§1.1). */
  conflicts: string[];
  /** Every dated read the extractor found, freshest first (detail view). */
  reads: DatedRead[];
  /** The section's usage prose, trimmed (detail view + shell cards). */
  usageNote?: string;
}

/** One line of the PAYG dollars card (§1.2). */
export interface PaygLine {
  id: string;
  label: string;
  /** Accrued this period, when a figure was recorded. */
  accruedUsd: number | null;
  collectedUsd?: number | null;
  estimate?: PaygEstimate | null;
  /** Unbilled accrual counted toward the $5 bubble (the counted figure). */
  unbilledUsd: number | null;
  note: string;
  measuredAt: string | null;
  sourceClass: UsageSourceClass;
}

/** One observed charge (§2.4 evidence — Plaid/Gmail/receipt class). */
export interface ChargeRow {
  /** ISO date the charge posted/was observed. */
  at: string;
  label: string;
  amountUsd: number;
  /** Payment method as recorded, e.g. "Visa •7535". */
  method: string;
  /** Evidence source for this row. */
  source: string;
}

/**
 * One tripped bubble rule (§1.3): a metric over 80% of its limit, or
 * projected unbilled PAYG over $5 (unbilled accrual only, Q3 default).
 */
export interface AttentionFlag {
  kind: "over-limit" | "payg-unbilled" | "unmatched-charge";
  label: string;
  detail: string;
  href: string;
  /** Percent (over-limit) or counted dollars (payg-unbilled); worst first. */
  severity: number;
}

/**
 * The quota plane's latest-state contract (Stage 2, #1151): one entry
 * per provider credential the gatherer read this tick, each window
 * carrying the hub's {value, pctOfLimit, resetsAt, measuredAt, source,
 * state} shape. Written by scripts/gather-quota.ts to
 * quota-current.json beside fleet-feed.json; the collector embeds it
 * in the feed as `quotaCurrent` for the laptop aggregator.
 */
export interface QuotaCurrentWindow {
  /** Window key from the provider result (e.g. "weekly", "5h", "credits_balance"). */
  name: string;
  /** Used value in the window's own unit: percent for plan windows, null for label-only rows. */
  value: number | null;
  /** Used percent of the window's limit, or null when the provider reports no percent. */
  pctOfLimit: number | null;
  /** Human label for non-percent values (e.g. a balance "¥44.29"), or null. */
  valueLabel: string | null;
  windowSeconds: number | null;
  resetsAt: string | null;
  measuredAt: string;
  source: string;
  state: "ok" | "error";
}

export interface QuotaCurrentEntry {
  providerId: string;
  providerName: string;
  /** Hub snapshot/pool service id this provider's records land under. */
  serviceId: string;
  credentialId?: string;
  credentialLabel?: string;
  planLabel?: string;
  state: "ok" | "error" | "unconfigured";
  measuredAt: string;
  source: string;
  error?: string;
  windows: QuotaCurrentWindow[];
  /** The provider result's raw usage object, kept for fidelity until consumers settle. */
  usage?: unknown;
}

/** One gatherer tick's health record, embedded in quota-current.json and kept standalone. */
export interface QuotaGatherHeartbeat {
  tickAt: string;
  ok: boolean;
  durationMs: number;
  perProvider: Record<string, { state: "ok" | "error"; ms: number; rows: number; error?: string }>;
  /** Set when the tick itself failed before any provider ran (import / credential store). */
  error?: string;
}

export interface QuotaCurrentFile {
  version: 1;
  generatedAt: string;
  source: string;
  heartbeat: QuotaGatherHeartbeat;
  providers: QuotaCurrentEntry[];
}

/**
 * One snapshot-store record (§4). Estimates are stored as a low/high
 * pair with unit "usd-est"; the 1.2× rule applies at render.
 */
export interface SnapshotRecord {
  /** ISO timestamp of the observation (its original date, for backfill). */
  ts: string;
  serviceId: string;
  metricId: string;
  value: number | { low: number; high: number };
  unit: "pct" | "usd" | "count" | "gb" | "tokens" | "usd-est" | "minutes" | "hours" | "gb-hours";
  limit?: number | null;
  resetsAt?: string | null;
  source: UsageSourceClass;
  observedBy: string;
}

/** The hub's family-2 + PAYG + charges model, served by Source.usage(). */
export interface UsageView {
  services: SaasService[];
  payg: PaygLine[];
  charges: ChargeRow[];
  /** Set when no charge evidence could be derived — the strip explains itself. */
  chargesNote?: string;
  attention: AttentionFlag[];
  /** Domains summary from the feed's dns section (§1.4: link, don't fork). */
  domains: { totalAnnualUsd: number; nextRenewal: string | null; domainCount: number } | null;
  /** Flat/renewal-only rows for the subscriptions strip (Q5 default). */
  subscriptions: { name: string; plan: string; renewsAt: string | null }[];
  /** Snapshot history per service id, from the §4 store (may be empty). */
  history: Record<string, SnapshotRecord[]>;
}
