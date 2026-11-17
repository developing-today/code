// The fleet feed: a JSON file produced on the laptop by
// scripts/collect-fleet-feed.ts, run by the laptop's fleet-feed-collector
// user timer (declared in the pm-stack NixOS module, #3363/#3368) and
// written straight to the aggregator's state dir. (The collector
// originally ran on the sandbox and pushed the feed here; that sandbox
// timer is retired — a stale sandbox copy overwriting this file was
// code#3362.) The aggregator renders the feed as-is; a stale feed keeps
// rendering with its age stated in provenance.
import { readFile } from "node:fs/promises";
import type { UnmatchedCharge } from "./charges-match.ts";
import type { CheckoutDrift } from "./checkout-drift.ts";
import { isProjectAudit, isProjectPoolItem } from "./projects.ts";
import { isRepoCensus, type RepoCensus } from "./coverage.ts";
import { isSessionRecord } from "./registry.ts";
import {
  arrayOf,
  isFiniteNumber,
  isIsoDate,
  isNonNegative,
  isPercentOf,
  isRecord,
  isStringArray,
  isTimestampNotFuture,
} from "./validate.ts";
import type {
  AaFeedSection,
  BurnSegment,
  ChargeRow,
  DnsSection,
  GoalMeasurement,
  OrchestrationProvenance,
  OrchestrationSection,
  PaygLine,
  PoolFreshness,
  ProjectPoolItem,
  ProjectAudit,
  QuotaCredentialRow,
  QuotaPool,
  QuotaView,
  QuotaWindow,
  SaasService,
  UninstrumentedSaasService,
  SessionFeedEntry,
  SessionRecord,
  SnapshotRecord,
} from "./model.ts";
import { isCredentialLifecycleRow, safeProviderReason } from "./credential-lifecycle.ts";

export interface FleetFeed {
  version: 1;
  /** ISO timestamp of collection on the sandbox. */
  generatedAt: string;
  /** Collector checkout drift vs origin/main (#1116 item 3), measured
   *  by the collector in its own checkout at collection time. Absent
   *  in feeds written before the drift check existed. */
  checkout?: CheckoutDrift;
  quota: QuotaView;
  /** Element and optional-field drops loadFeed's quota sanitization made
   * (#2429/#3921/#4546), set
   * only on feeds returned by loadFeed; absent on raw parsed feeds. */
  quotaDropped?: {
    pools: number;
    credentials: number;
    windows: number;
    activeSessions: number;
    burn: number;
    poolFreshness: number;
    burnTokensPerMin: number;
    credentialLifecycle?: number;
  };
  /** Element drops loadFeed's chargesAttention sanitization made
   * (#2439), set only when at least one entry dropped; absent on raw
   * parsed feeds and on feeds whose entries all survived. */
  chargesAttentionDropped?: number;
  /** Subscription rows loadFeed's sanitization dropped (#2446), set
   * only when at least one row (or a non-array section) was dropped;
   * absent on raw parsed feeds and on clean loads. */
  subscriptionsDropped?: number;
  /** Registry rows loadFeed's sanitization dropped (#2539), set only
   * when at least one row (or a non-array section) was dropped. */
  registryDropped?: number;
  /** Rows loadFeed's charges sanitization dropped (#2445), set only
   * when drops occurred; absent on raw parsed feeds and clean loads. */
  chargesDropped?: number;
  /** SaaS service rows loadFeed's sanitization dropped (#2451), set
   * only when drops occurred; absent on raw parsed feeds and clean loads. */
  saasDropped?: number;
  /** PAYG lines loadFeed's sanitization dropped (#2450), set only
   * when drops occurred; absent on raw parsed feeds and clean loads. */
  paygDropped?: number;
  /** Sandbox-server sessions, pre-classification (render classifies). */
  attention: SessionFeedEntry[];
  /** The orchestration job's latest section (#899), when one exists. */
  orchestration?: OrchestrationSection;
  /** The DNS/domain register (#902), parsed by the collector (slice 3). */
  dns?: DnsSection;
  /** Identity-registry records (#906 §3): sandbox OpenCode + Muse launches. */
  registry?: SessionRecord[];
  /** Usage & billing hub (design §2.3): family-2 services extracted from
   * quota-view.md Part (b) by the collector; absent in older feeds. */
  saas?: SaasService[];
  /** Tracker-derived services without an instrumented quota card. */
  uninstrumentedServices?: UninstrumentedSaasService[];
  /** Malformed tracker rows dropped by loadFeed (#1172). */
  uninstrumentedServicesDropped?: number;
  /** Hub PAYG lines (§1.2), from quota-view.md + the provider registry. */
  payg?: PaygLine[];
  /** Hub charges evidence (§2.4), from the same dated records. */
  charges?: ChargeRow[];
  /** Charges auto-matching (#1057): unmatched SaaS/AI-looking charges
   * surfacing for the FIRST time this run — the collector filters out
   * anything its persisted surfaced-state already showed, so a charge
   * appears here exactly once and the live source turns each entry
   * into a hub attention flag. Absent when nothing new surfaced. */
  chargesAttention?: UnmatchedCharge[];
  /** Hub subscriptions strip rows (Q5: renewal-only). */
  subscriptions?: { name: string; plan: string; renewsAt: string | null }[];
  /** Usage snapshot records (hub §4 writers): derived by the collector
   * from this feed's own hub sections plus the writers' outbox, for
   * the laptop aggregator to ingest into its local snapshot store.
   * Absent in older feeds; the store dedupes, so republication of the
   * same observations appends nothing. */
  snapshots?: SnapshotRecord[];
  /** GitHub Project pool (#910 stage 5): every item of the org Project
   * "Developing Today" (#4) with its Project fields and linked issue
   * state, read by the collector via GraphQL. Absent when the
   * collector had no Projects token or the read failed. */
  projects?: ProjectPoolItem[];
  /** Scheduled Project #4 audit snapshot (fields, views, totals, and distributions). */
  projectAudit?: ProjectAudit;
  /** Project #4 coverage census (#1220/#1224): per-repo open-issue,
   * gate-label, and timestamped >90-day untouched-issue counts from the
   * collector's Search API reads, embedded beside the projects section.
   * Absent when the collector had no GitHub token or the census failed. */
  coverage?: RepoCensus[];
  /** AA cost explorer section: the ainventory (AA import) data read
   * by the collector from Supabase, plus pool/catalog coverage computed
   * against the repo's opencode config. Present-but-absent
   * ({status:"absent", reason}) when Supabase was unreachable — the
   * section degrades, the feed never fails over it. */
  aa?: AaFeedSection;
  /** Optional collector outcomes. Older feeds have no metadata and keep
   *  their historical presence-only health/provenance behavior. */
  meta?: FeedMetadata;
}

export type FeedSectionStatus = "ok" | "error" | "skipped";

export interface FeedSectionCollectionStatus {
  status: FeedSectionStatus;
  /** When this collection attempt finished (ISO timestamp). */
  at: string;
  /** Sanitized diagnostic for an error outcome only. */
  error?: string;
}

export interface FeedMetadata {
  sections: Record<string, FeedSectionCollectionStatus>;
}

// Cap `t` at `max` UTF-16 code units, but never leave half of a surrogate
// pair at the cut (#7741, mirroring the #7694 capUnits guard in actions.ts):
// an astral character straddling the cap is dropped whole. Local copy, like
// comments.ts (#7731) — a shared import would tangle module dependencies.
function capUnits(t: string, max: number): string {
  if (t.length <= max) return t;
  let end = max;
  const before = t.charCodeAt(end - 1);
  const after = t.charCodeAt(end);
  if (before >= 0xd800 && before <= 0xdbff && after >= 0xdc00 && after <= 0xdfff) end--;
  return t.slice(0, end);
}

/** Keep collector diagnostics useful without allowing credentials or
 *  response payloads to leak into the shared feed. */
export function sanitizeCollectionError(value: unknown): string {
  let message = value instanceof Error ? value.message : typeof value === "string" ? value : "collection failed";
  message = message
    .replace(/\bBearer\s+[^\s,;]+/gi, "Bearer [redacted]")
    .replace(/\bBasic\s+[A-Za-z0-9+/=]+/gi, "Basic [redacted]")
    .replace(/\b(?:gh[pousr]_[A-Za-z0-9_]{8,}|github_pat_[A-Za-z0-9_]+|xox[baprs]-[A-Za-z0-9-]+)/gi, "[redacted]")
    .replace(/\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b/g, "[redacted]")
    .replace(
      // Quoted keys (#3966): JSON bodies embed credentials as
      // {"api_key": "v"}, {'password': 'v'}, or nested {"secret":
      // "v"} — the keyword is wrapped in quotes, so allow an
      // optional quote on each side of the name. `key` covers bare
      // URL credential params (?key=..., &api_key=...); the value
      // stops at & so sibling params survive.
      /(["']?\b(?:access[_-]?token|refresh[_-]?token|token|api[_-]?key|password|secret|key|authorization)["']?\s*[:=]\s*)(?:"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|[^\s,;&]+)/gi,
      "$1[redacted]",
    )
    .replace(/(https?:\/\/)[^/@\s:]+(?::[^/@\s]*)?@/gi, "$1[redacted]@")
    .replace(/\b[A-Za-z0-9_-]{40,}\b/g, "[redacted]")
    .replace(/[\r\n\t]+/g, " ");
  message = capUnits(message, 240);
  return message || "collection failed";
}

export function feedSectionStatus(status: FeedSectionStatus, at: string, error?: unknown): FeedSectionCollectionStatus {
  return status === "error" ? { status, at, error: sanitizeCollectionError(error) } : { status, at };
}

/** Tolerate malformed optional metadata without rejecting an otherwise
 *  usable feed. The same boundary also sanitizes error text from older
 *  or externally-written collectors. */
export function sanitizeFeedMetadata(value: unknown): FeedMetadata | undefined {
  if (!isRecord(value) || !isRecord(value.sections)) return undefined;
  const sections: FeedMetadata["sections"] = {};
  for (const [key, raw] of Object.entries(value.sections)) {
    if (!/^[A-Za-z][A-Za-z0-9_-]{0,63}$/.test(key) || !isRecord(raw)) continue;
    const at = raw.at;
    if (
      (raw.status !== "ok" && raw.status !== "error" && raw.status !== "skipped") ||
      typeof at !== "string" ||
      !isTimestampNotFuture(at)
    ) {
      continue;
    }
    sections[key] = feedSectionStatus(raw.status, at, raw.error);
  }
  return Object.keys(sections).length ? { sections } : undefined;
}

/**
 * Minimal structural check for an orchestration section, shared by the
 * collector (which embeds the orchestrator's file fail-soft) and the
 * live source (which serves it). Absent or malformed = "not measured".
 */
/**
 * Per-goal gate (#949): `todayFromSection` dereferences `linedUp`,
 * `stalled`, and `gaps` on every goal, so a goal missing any of them must
 * fail validation here — this validator is the only gate on every path,
 * and a rejected section falls back to the derived view by design.
 * `progressedSinceLastRun` is deliberately string-or-array: `asStringList`
 * in derive.ts normalizes the single-string form the orchestrator
 * sometimes publishes, so requiring strictly an array would reject real
 * sections.
 */
const GOAL_MOMENTUMS: readonly string[] = ["moving", "stalled", "blocked", "no-work-lined-up"];
const LINED_UP_HORIZONS: readonly string[] = ["today", "tomorrow", "next-3-days", "longer", "incoming"];
const LINED_UP_STATUSES: readonly string[] = ["in-progress", "queued", "blocked", "done-recently"];

// #3897: element gates for the per-goal arrays. todayFromSection reads
// element fields directly (t.horizon, s.blocker), so a record alone is
// not enough — the fields it dereferences must be the right shapes, or
// the designed invalid-section fallback never happens and the consumer
// throws (or silently renders falsified state) instead.
function isLinedUpElement(v: unknown): boolean {
  if (!isRecord(v)) return false;
  return (
    typeof v.title === "string" &&
    typeof v.horizon === "string" &&
    LINED_UP_HORIZONS.includes(v.horizon) &&
    (v.status === undefined || (typeof v.status === "string" && LINED_UP_STATUSES.includes(v.status))) &&
    (v.horizonWhy === undefined || typeof v.horizonWhy === "string") &&
    // #6174: consumers dereference `ref` as a string (assembleWorkItems
    // keys items by it; todayFromSection carries it to the render), so
    // its type is gated here like the other dereferenced fields. A
    // string that is not a GitHub ref stays valid at the gate — goal
    // ids are legitimate lined-up refs in live data (#6149); consumers
    // guard those with isValidRef at the render (#6149/#6205).
    (v.ref === undefined || typeof v.ref === "string") &&
    // #4316: blockedBy is a DependencyRef[] — the same shape the
    // top-level dependencies array is gated with (isDependencyElement).
    // Ungated, a malformed blockedBy passed the section as valid and
    // its blockers were silently dropped by the canonical layer
    // (asBlockers), rendering a blocked task as unblocked. Absence is
    // fine; a present value must be an array of dependency elements.
    (v.blockedBy === undefined ||
      (Array.isArray(v.blockedBy) && v.blockedBy.every(isDependencyElement)))
  );
}

function isStalledElement(v: unknown): boolean {
  if (!isRecord(v)) return false;
  return (
    typeof v.title === "string" &&
    typeof v.since === "string" &&
    (v.blocker === undefined || typeof v.blocker === "string")
  );
}

function isGoalMeasurement(v: unknown): boolean {
  if (typeof v !== "object" || v === null) return false;
  const g = v as Partial<GoalMeasurement>;
  return (
    // #3897: the scalar fields todayFromSection renders (project name,
    // momentum state, summary) were never checked — a goal missing them
    // passed and rendered nameless / silently "stalled".
    typeof g.goal === "string" &&
    g.goal.length > 0 &&
    typeof g.momentum === "string" &&
    GOAL_MOMENTUMS.includes(g.momentum) &&
    typeof g.summary === "string" &&
    (typeof g.goalId === "string" || g.goalId === null) &&
    Array.isArray(g.linedUp) &&
    Array.isArray(g.stalled) &&
    Array.isArray(g.gaps) &&
    // #2417: todayFromSection dereferences every lined-up / stalled
    // element, so a non-record element must fail here, at the gate —
    // presence of the arrays alone let [null] through to a TypeError.
    g.linedUp.every(isLinedUpElement) &&
    g.stalled.every(isStalledElement) &&
    // #3897: gaps are joined into the rendered detail line, so a
    // non-string element would render as "[object Object]".
    g.gaps.every((gap) => typeof gap === "string") &&
    (typeof g.progressedSinceLastRun === "string" ||
      (Array.isArray(g.progressedSinceLastRun) && g.progressedSinceLastRun.every((line) => typeof line === "string")))
  );
}

/**
 * Element gates for the section's remaining arrays (#2417). The
 * section gate used to check only that these arrays exist; poisoned
 * elements (null, primitives, shapeless objects) then reached
 * consumers that dereference them — buildAttention on tier items,
 * the projects view on dependencies, the today view on actions —
 * and threw TypeErrors instead of taking the designed invalid-section
 * fallback. The rules stay at the depth consumers need:
 *
 * - Tier items: a plain record with a string `id`. Deeper fields are
 *   the per-item sanitizer's job (src/tier-items.ts, #1290) — a
 *   malformed extension drops there, never the item, so the gate
 *   must not reject on them.
 * - Dependencies: the declared consumed shape — required string
 *   `task`, `status` in the DependencyRef union, and the optional
 *   ref/worker fields string-typed when present.
 * - Actions: the four rendered string fields (at, kind, target,
 *   summary).
 */
function isTierElement(v: unknown): boolean {
  return isRecord(v) && typeof v.id === "string";
}

const DEPENDENCY_STATUSES: readonly string[] = ["in-progress", "queued", "stalled", "done", "unknown"];

function isDependencyElement(v: unknown): boolean {
  if (!isRecord(v)) return false;
  return (
    typeof v.task === "string" &&
    typeof v.status === "string" &&
    DEPENDENCY_STATUSES.includes(v.status) &&
    (v.ref === undefined || typeof v.ref === "string") &&
    (v.taskRef === undefined || typeof v.taskRef === "string") &&
    (v.workerSessionId === undefined || typeof v.workerSessionId === "string") &&
    (v.workerLabel === undefined || typeof v.workerLabel === "string")
  );
}

function isActionElement(v: unknown): boolean {
  return (
    isRecord(v) &&
    typeof v.at === "string" &&
    typeof v.kind === "string" &&
    typeof v.target === "string" &&
    typeof v.summary === "string"
  );
}

/**
 * The section's provenance (#1110). The contract is the publisher's
 * object form — `{ sources: string[], runId?: string }` — which every
 * orchestration run actually emits. The legacy plain-string form is
 * also accepted here and normalized at the boundary (see
 * normalizeOrchestrationSection), the same string-or-richer tolerance
 * derive.ts gives `progressedSinceLastRun`. Anything else — a number,
 * an object without a sources array, non-string sources — fails.
 */
function isProvenance(v: unknown): boolean {
  if (typeof v === "string") return v.length > 0;
  if (typeof v !== "object" || v === null) return false;
  const p = v as { sources?: unknown; runId?: unknown };
  return (
    Array.isArray(p.sources) &&
    p.sources.every((x) => typeof x === "string") &&
    (p.runId === undefined || typeof p.runId === "string")
  );
}

/**
 * The first structural check an orchestration section fails, as a short
 * field name — or null when the section is valid (#1140: a present-but-
 * invalid section must be diagnosable, not silently "absent"). This is
 * the single source of truth behind isOrchestrationSection, so the
 * diagnosis can never drift from the gate itself.
 */
export function orchestrationSectionProblem(v: unknown): string | null {
  if (typeof v !== "object" || v === null) return "not an object";
  const s = v as Partial<OrchestrationSection>;
  if (s.version !== 1) return "version";
  if (!isIsoDate(s.generatedAt)) return "generatedAt";
  if (typeof s.runId !== "string") return "runId";
  if (!isProvenance(s.provenance)) return "provenance";
  if (typeof s.tiers !== "object" || s.tiers === null) return "tiers";
  if (!Array.isArray(s.tiers.needsDrew)) return "tiers.needsDrew";
  if (!s.tiers.needsDrew.every(isTierElement)) return "tiers.needsDrew (per-item shape)";
  if (!Array.isArray(s.tiers.needsOrchestrator)) return "tiers.needsOrchestrator";
  if (!s.tiers.needsOrchestrator.every(isTierElement)) return "tiers.needsOrchestrator (per-item shape)";
  if (!Array.isArray(s.dependencies)) return "dependencies";
  if (!s.dependencies.every(isDependencyElement)) return "dependencies (per-dependency shape)";
  if (!Array.isArray(s.goals)) return "goals";
  if (!s.goals.every(isGoalMeasurement)) return "goals (per-goal shape)";
  if (!Array.isArray(s.actions)) return "actions";
  if (!s.actions.every(isActionElement)) return "actions (per-action shape)";
  return null;
}

export function isOrchestrationSection(v: unknown): v is OrchestrationSection {
  return orchestrationSectionProblem(v) === null;
}

/**
 * A section older than this at the hourly cadence is stale (#1139,
 * #1148's threshold): still served, but every surface that renders its
 * numbers must say the data is old instead of presenting it as current.
 */
export const ORCHESTRATION_STALE_MS = 90 * 60_000;

// The collector is scheduled every five minutes. No grace is possible if a
// stop just after a write must be visible by the next interval: slow runs,
// scheduler jitter and transport delays can therefore cause transient alarms.
// This is evaluated on request, not pushed to an already-open browser.
export const FEED_STALE_MS = 5 * 60_000;

/** Future-skew tolerance for generatedAt stamps (#1564; the aggregator-wide 60s convention ruled for #1541). */
export const GENERATED_AT_FUTURE_SKEW_MS = 60_000;

/**
 * The diagnosis behind the honesty surfaces (#1139/#1140): a section
 * the aggregator holds is in exactly one of these states. "invalid"
 * names the failing checks (from orchestrationSectionProblem — the
 * gate itself, so the diagnosis can never drift from the validation)
 * and is never collapsed into "absent": a publisher whose section is
 * being rejected has a bug to fix, not a run that never happened.
 */
export interface SectionDiagnosis {
  state: "live" | "absent" | "invalid" | "stale";
  generatedAt?: string;
  runId?: string;
  ageMs?: number;
  /** Names of the failing structural checks; never values. */
  failedChecks?: string[];
}

export function diagnoseOrchestrationSection(v: unknown, now = Date.now()): SectionDiagnosis {
  if (v === null || v === undefined) return { state: "absent" };
  const problem = orchestrationSectionProblem(v);
  if (problem !== null) return { state: "invalid", failedChecks: [problem] };
  const s = v as OrchestrationSection;
  const parsedAt = Date.parse(s.generatedAt);
  if (parsedAt - now > GENERATED_AT_FUTURE_SKEW_MS) {
    return { state: "invalid", failedChecks: ["generatedAt"], generatedAt: s.generatedAt, runId: s.runId };
  }
  const ageMs = Math.max(0, now - parsedAt);
  return {
    state: ageMs > ORCHESTRATION_STALE_MS ? "stale" : "live",
    generatedAt: s.generatedAt,
    runId: s.runId,
    ageMs,
  };
}

/**
 * A valid section with its provenance in the canonical object form:
 * a legacy plain-string provenance becomes `{ sources: [it] }`.
 * Returns null for anything isOrchestrationSection rejects. The
 * collector embeds this normalized form in the feed and the live
 * source serves it, so downstream consumers see exactly one shape.
 */
export function normalizeOrchestrationSection(v: unknown): OrchestrationSection | null {
  if (!isOrchestrationSection(v)) return null;
  const s = v as OrchestrationSection & { provenance: OrchestrationProvenance | string };
  if (typeof s.provenance !== "string") return s;
  return { ...s, provenance: { sources: [s.provenance] } };
}

/**
 * The first structural problem in a feed's GitHub Project pool section
 * (#2399, part 1 of #1122) — or null when the section is valid. The
 * per-item rule is isProjectPoolItem, the same predicate the collector
 * parses against and the live source filters with, so the feed check
 * and the collector can never drift: edge fields (parentRef,
 * blockedBy, blocks, epicRef) are absence-friendly but shape-strict
 * when present. A present-but-invalid section must be diagnosable, not
 * silently "absent" (#1140) — but a single malformed item still only
 * drops that item (see poolFromFeed), never the pool.
 */
export function projectPoolSectionProblem(v: unknown): string | null {
  if (v === undefined) return null;
  if (!Array.isArray(v)) return "not an array";
  for (let i = 0; i < v.length; i++) {
    if (!isProjectPoolItem(v[i])) return `item ${i}`;
  }
  return null;
}

/** A valid projects section: absent (undefined) or an array whose
 * every item passes the shared pool-item gate. */
export function isProjectPoolSection(v: unknown): v is ProjectPoolItem[] | undefined {
  return projectPoolSectionProblem(v) === null;
}

/** Fail-soft read of the pool from a parsed feed: absent or malformed
 * sections yield no items, and malformed items drop individually — the
 * single gate both live-source call sites share. */
export function poolFromFeed(feed: Partial<FleetFeed> | null | undefined): ProjectPoolItem[] {
  const raw = feed?.projects;
  if (raw === undefined) return [];
  if (!Array.isArray(raw)) return [];
  return raw.filter(isProjectPoolItem);
}

/** Whether the feed actually carries a projects section. The collector
 * omits the section entirely when its Project read fails, so presence
 * is the availability signal: an absent (or malformed) section means
 * the pool is unavailable, which coverage must not render as zero
 * items (#1220, PR #3309). A present array is available even when
 * individual malformed items drop in poolFromFeed. */
export function poolAvailableFromFeed(feed: Partial<FleetFeed> | null | undefined): boolean {
  return Array.isArray(feed?.projects);
}

/** Fail-soft read of the scheduled Project #4 audit snapshot. */
export function projectAuditFromFeed(feed: Partial<FleetFeed> | null | undefined): ProjectAudit | null {
  return isProjectAudit(feed?.projectAudit) ? feed.projectAudit : null;
}

/** Fail-soft read of the coverage census from a parsed feed: absent
 * or malformed sections yield null (the panel then renders backlog
 * counts as unavailable), malformed rows drop individually. A duplicate
 * repo makes the census ambiguous, so refuse that optional section
 * rather than letting array order choose the persisted count. */
export function coverageFromFeed(feed: Partial<FleetFeed> | null | undefined): RepoCensus[] | null {
  const raw = feed?.coverage;
  if (raw === undefined) return null;
  if (!Array.isArray(raw)) return null;
  const rows = raw.filter(isRepoCensus);
  const seen = new Set<string>();
  for (const row of rows) {
    if (seen.has(row.repo)) return null;
    seen.add(row.repo);
  }
  return rows.length ? rows : null;
}

const isQuotaPercent = isPercentOf(100);

/** One quota window as the renderers dereference it (#2429): `limit`
 * is read arithmetically by windowPercent, `used` is rendered, and a
 * non-object element crashes both renderQuota and buildAttentionFlags.
 * Value rules (#3427): a non-null resetsAt must be a real ISO calendar
 * date (impossible dates roll forward in guidance), non-null limit and
 * used must be non-negative finite, and percent when present must be
 * within 0..100 (negative or >100 percents misread as healthy/walled). */
export function isQuotaWindow(v: unknown): v is QuotaWindow {
  if (!isRecord(v)) return false;
  return (
    typeof v.name === "string" &&
    (v.unit === "requests" || v.unit === "tokens" || v.unit === "usd" || v.unit === "messages" || v.unit === "other") &&
    (v.limit === null || isNonNegative(v.limit)) &&
    isNonNegative(v.used) &&
    (v.resetsAt === null || isIsoDate(v.resetsAt)) &&
    (v.percent === undefined || isQuotaPercent(v.percent))
  );
}

const QUOTA_STATES = ["healthy", "near", "walled", "error", "stale"];

/** One quota pool whose own shape is safe to hand to the renderers
 * (#2429): an object with a `windows` ARRAY — a non-array windows value
 * must never be iterated as a collection of windows (it fabricated
 * attention flags). Window ELEMENTS are filtered by sanitizeQuotaView,
 * so a pool survives one bad window. Optional credential containers
 * must be arrays; malformed credential rows and active-session entries
 * are dropped individually by sanitizeQuotaView. */
export function isQuotaPool(v: unknown): v is QuotaPool {
  if (!isRecord(v)) return false;
  if (typeof v.id !== "string" || typeof v.provider !== "string" || typeof v.plan !== "string") return false;
  if (typeof v.state !== "string" || !QUOTA_STATES.includes(v.state)) return false;
  if (!Array.isArray(v.windows)) return false;
  if (v.credentials !== undefined && !Array.isArray(v.credentials)) return false;
  return true;
}

const QUOTA_FRESHNESS_STATES = ["ok", "stale", "error"];

function isQuotaFreshness(v: unknown): v is PoolFreshness {
  return (
    isRecord(v) &&
    isIsoDate(v.measuredAt) &&
    typeof v.source === "string" &&
    typeof v.state === "string" &&
    QUOTA_FRESHNESS_STATES.includes(v.state) &&
    (v.error === undefined || typeof v.error === "string")
  );
}

function isQuotaCredentialRow(v: unknown): v is QuotaCredentialRow {
  return (
    isRecord(v) &&
    typeof v.label === "string" &&
    typeof v.providerId === "string" &&
    Array.isArray(v.windows) &&
    isQuotaFreshness(v.freshness)
  );
}

type QuotaActiveSession = NonNullable<QuotaPool["activeSessions"]>[number];

function isQuotaActiveSession(v: unknown): v is QuotaActiveSession {
  return isRecord(v) && typeof v.session === "string" && typeof v.model === "string" && isNonNegative(v.tokens);
}

/** One historical burn segment as renderQuota maps it (#2429).
 *  Values, not just types (#3428): from/to must be real ISO calendar
 *  dates, tokens may not be negative, and the range may not be
 *  inverted — a segment failing any of these is not measured history
 *  and drops individually in sanitizeQuotaView (#2429 counting). */
export function isBurnSegment(v: unknown): v is BurnSegment {
  if (!isRecord(v)) return false;
  return (
    typeof v.provider === "string" &&
    typeof v.pool === "string" &&
    isIsoDate(v.from) &&
    isIsoDate(v.to) &&
    isNonNegative(v.tokens) &&
    Date.parse(v.from) <= Date.parse(v.to)
  );
}

/** One unmatched-charge entry as the live source dereferences it
 * (#2439): merchant and at are interpolated, amountUsd is formatted
 * with toFixed and used as a severity, so those three must be a
 * string, a finite number, and a string. normalized/source are not
 * gate-required: the consumer tolerates any source value and never
 * dereferences normalized. (#3458): amountUsd must be positive and at
 * must be a real ISO calendar date — impossible rows alter charge
 * totals and attention severity. */
export function isUnmatchedCharge(v: unknown): v is UnmatchedCharge {
  if (!isRecord(v)) return false;
  return typeof v.merchant === "string" && isFiniteNumber(v.amountUsd) && v.amountUsd > 0 && isIsoDate(v.at);
}

/** One charges-evidence row as the /quota charges strip renders it
 * (#2445): a record with string at/label/method/source and a
 * finite-number amountUsd (ChargeRow, model.ts). Built on the shared
 * validate.ts primitives, like the quota element validators above.
 * (#3458): amountUsd must be positive and at must be a real ISO
 * calendar date — impossible rows alter charge totals and attention
 * severity. */
export function isChargeRow(v: unknown): v is ChargeRow {
  if (!isRecord(v)) return false;
  return (
    isIsoDate(v.at) &&
    typeof v.label === "string" &&
    isFiniteNumber(v.amountUsd) &&
    v.amountUsd > 0 &&
    typeof v.method === "string" &&
    typeof v.source === "string"
  );
}

const SAAS_GROUPS = ["database", "object-storage", "compute", "dev-platform", "domains", "account-tooling"];
const USAGE_UNITS = [
  "pct",
  "usd",
  "cny",
  "count",
  "gb",
  "tokens",
  "minutes",
  "hours",
  "ms",
  "rows",
  "ops",
  "neurons",
  "other",
];
const USAGE_SOURCE_CLASSES = ["api", "console", "manual", "estimate", "invoice"];

function isUsageMetric(v: unknown): boolean {
  if (!isRecord(v)) return false;
  return (
    typeof v.id === "string" &&
    typeof v.label === "string" &&
    // Range gates mirror isSnapshotRecord (#5188): a negative used,
    // limit, or percent is never legitimate usage data, and a record
    // derived from one is refused on snapshot readback — refuse it
    // here, at the feed boundary, instead.
    (v.used === null || isNonNegative(v.used)) &&
    (v.limit === null || isNonNegative(v.limit)) &&
    typeof v.unit === "string" &&
    USAGE_UNITS.includes(v.unit) &&
    (v.percent === undefined || isNonNegative(v.percent)) &&
    (v.lowerBound === undefined || typeof v.lowerBound === "boolean") &&
    typeof v.highlight === "boolean" &&
    (v.note === undefined || typeof v.note === "string")
  );
}

function isPaygEstimate(v: unknown): boolean {
  if (!isRecord(v)) return false;
  const { low, high } = v;
  return (
    // Order + range gates mirror the snapshot pair rule (#5188):
    // endpoints are non-negative and low <= high (low === high and
    // zero endpoints pass); an inverted or negative pair renders
    // nonsense money and its derived snapshot record is refused by
    // isSnapshotRecord.
    isNonNegative(low) &&
    isNonNegative(high) &&
    low <= high &&
    typeof v.period === "string" &&
    (v.note === undefined || typeof v.note === "string")
  );
}

function isSaasService(v: unknown): v is SaasService {
  if (!isRecord(v)) return false;
  const sources = v.sources;
  const sourcesValid =
    sources === undefined ||
    (isRecord(sources) &&
      typeof sources.quotaViewSection === "string" &&
      (typeof sources.trackerRow === "string" || sources.trackerRow === null) &&
      typeof sources.baselineDoc === "string");
  return (
    typeof v.id === "string" &&
    typeof v.name === "string" &&
    typeof v.group === "string" &&
    SAAS_GROUPS.includes(v.group) &&
    typeof v.plan === "string" &&
    (v.status === "active" || v.status === "candidate" || v.status === "no-account" || v.status === "unknown") &&
    Array.isArray(v.metrics) &&
    v.metrics.every(isUsageMetric) &&
    (v.paygEstimate === null || isPaygEstimate(v.paygEstimate)) &&
    (v.paygNote === undefined || typeof v.paygNote === "string") &&
    (v.billedUsd === undefined || v.billedUsd === null || isNonNegative(v.billedUsd)) &&
    (v.measuredAt === null || isIsoDate(v.measuredAt)) &&
    typeof v.sourceClass === "string" &&
    USAGE_SOURCE_CLASSES.includes(v.sourceClass) &&
    typeof v.sourceNote === "string" &&
    sourcesValid &&
    (v.nextDate === undefined || typeof v.nextDate === "string" || v.nextDate === null) &&
    isNonNegative(v.cadenceDays) &&
    isStringArray(v.conflicts) &&
    Array.isArray(v.reads) &&
    v.reads.every((read) => isRecord(read) && typeof read.at === "string" && typeof read.text === "string") &&
    (v.usageNote === undefined || typeof v.usageNote === "string")
  );
}

export interface SaasSanitization {
  /** Renderer-safe services, or undefined when the optional section is absent/non-array. */
  rows?: SaasService[];
  /** Invalid array elements, or one when a present section is not an array. */
  dropped: number;
}

/** Drop malformed service rows at the feed boundary (#2451). One bad
 * service must not take down usage(); non-arrays are treated as absent
 * so they cannot silently choose the feed usage branch. */
export function sanitizeSaasServices(v: unknown): SaasSanitization {
  if (v === undefined) return { dropped: 0 };
  if (!Array.isArray(v)) return { dropped: 1 };
  const rows = v.filter(isSaasService);
  return { rows, dropped: v.length - rows.length };
}

/** The choices block renderQuota dereferences (hasHeadroom et al.). */
function isQuotaChoices(v: unknown): boolean {
  if (!isRecord(v)) return false;
  return isStringArray(v.hasHeadroom) && isStringArray(v.burnBeforeReset) && isStringArray(v.doNotStart);
}

export interface QuotaSanitization {
  view: QuotaView;
  droppedPools: number;
  droppedCredentials: number;
  droppedWindows: number;
  droppedActiveSessions: number;
  droppedBurn: number;
  droppedPoolFreshness: number;
  droppedBurnTokensPerMin: number;
  droppedCredentialLifecycle: number;
}

/** Drop-and-count sanitization of a gate-passed quota section (#2429/#3921/#4546,
 * following poolFromFeed's per-element pattern, built on the shared
 * arrayOf primitive from #2577): malformed pools and burn segments
 * drop individually, malformed windows/credential rows/active sessions
 * drop out of their pool, and the well-formed remainder renders. The
 * counts ride out on the result so the caller surfaces them, never
 * swallows them. */
export function sanitizeQuotaView(quota: QuotaView): QuotaSanitization {
  const pools = arrayOf(isQuotaPool)(quota.pools);
  let droppedCredentials = 0;
  let droppedWindows = 0;
  let droppedActiveSessions = 0;
  let droppedPoolFreshness = 0;
  let droppedBurnTokensPerMin = 0;
  const lifecycleRows =
    quota.credentialLifecycle === undefined ? undefined : arrayOf(isCredentialLifecycleRow)(quota.credentialLifecycle);
  const droppedCredentialLifecycle = lifecycleRows?.dropped ?? (quota.credentialLifecycle === undefined ? 0 : 1);
  const keptPools: QuotaPool[] = (pools?.values ?? []).map((pool) => {
    const windows = arrayOf(isQuotaWindow)(pool.windows);
    droppedWindows += windows?.dropped ?? 0;
    let out: QuotaPool = { ...pool, windows: windows?.values ?? [] };
    if (pool.freshness !== undefined && !isQuotaFreshness(pool.freshness)) {
      delete out.freshness;
      droppedPoolFreshness += 1;
    }
    if (pool.burnTokensPerMin !== undefined && !isNonNegative(pool.burnTokensPerMin)) {
      delete out.burnTokensPerMin;
      droppedBurnTokensPerMin += 1;
    }
    if (pool.credentials !== undefined) {
      const rows = arrayOf(isQuotaCredentialRow)(pool.credentials);
      droppedCredentials += rows?.dropped ?? 0;
      const credentials = (rows?.values ?? []).map((row) => {
        const rowWindows = arrayOf(isQuotaWindow)(row.windows);
        droppedWindows += rowWindows?.dropped ?? 0;
        return { ...row, windows: rowWindows?.values ?? [] };
      });
      out = { ...out, credentials };
    }
    if (pool.activeSessions !== undefined) {
      if (Array.isArray(pool.activeSessions)) {
        const sessions = arrayOf(isQuotaActiveSession)(pool.activeSessions);
        droppedActiveSessions += sessions?.dropped ?? 0;
        out = { ...out, activeSessions: sessions?.values ?? [] };
      } else {
        // A present malformed section is one dropped section, not a pool
        // failure; the renderer treats an empty list as no active sessions.
        droppedActiveSessions += 1;
        out = { ...out, activeSessions: [] };
      }
    }
    return out;
  });
  const burn = arrayOf(isBurnSegment)(quota.burn);
  const droppedPools = pools?.dropped ?? 0;
  const droppedBurn = burn?.dropped ?? 0;
  // Guidance must describe the surviving usable pools only: the
  // retained choices were computed over the pre-sanitization pools,
  // and computeChoices() (quota-guidance.ts) gives no guidance for a
  // pool with no windows — so filter every list to kept pools that
  // still have windows. A pool whose data was discarded must not
  // stay recommended under hasHeadroom or burnBeforeReset.
  const usable = new Set(keptPools.filter((p) => p.windows.length > 0).map((p) => p.id));
  const choices = {
    hasHeadroom: quota.choices.hasHeadroom.filter((id) => usable.has(id)),
    burnBeforeReset: quota.choices.burnBeforeReset.filter((id) => usable.has(id)),
    doNotStart: quota.choices.doNotStart.filter((id) => usable.has(id)),
  };
  const view: QuotaView = {
    ...quota,
    pools: keptPools,
    burn: burn?.values ?? [],
    choices,
    ...(lifecycleRows
      ? {
          credentialLifecycle: lifecycleRows.values.map((row) => ({
            ...row,
            providerName: safeProviderReason(row.providerName),
            label: safeProviderReason(row.label),
            ...(row.reason ? { reason: safeProviderReason(row.reason) } : {}),
            ...(row.userAction
              ? { userAction: { owner: "user" as const, title: safeProviderReason(row.userAction.title) } }
              : {}),
          })),
        }
      : {}),
  };
  // Surface the loss on the view itself (its consumers project the
  // view, e.g. /api/quota.json), not only on the feed wrapper.
  if (
    droppedPools +
      droppedCredentials +
      droppedWindows +
      droppedActiveSessions +
      droppedBurn +
      droppedPoolFreshness +
      droppedBurnTokensPerMin +
      droppedCredentialLifecycle >
    0
  ) {
    view.dropped = {
      pools: droppedPools,
      credentials: droppedCredentials,
      windows: droppedWindows,
      activeSessions: droppedActiveSessions,
      burn: droppedBurn,
      poolFreshness: droppedPoolFreshness,
      burnTokensPerMin: droppedBurnTokensPerMin,
      ...(droppedCredentialLifecycle > 0 ? { credentialLifecycle: droppedCredentialLifecycle } : {}),
    };
  }
  return {
    view,
    droppedPools,
    droppedCredentials,
    droppedWindows,
    droppedActiveSessions,
    droppedBurn,
    droppedPoolFreshness,
    droppedBurnTokensPerMin,
    droppedCredentialLifecycle,
  };
}

export function isFleetFeed(v: unknown): v is FleetFeed {
  if (typeof v !== "object" || v === null) return false;
  const f = v as Partial<FleetFeed>;
  if (!(typeof f.quota === "object" && f.quota !== null)) return false;
  const q = f.quota as QuotaView;
  return (
    f.version === 1 &&
    // A generatedAt implausibly in the future is invalid, not
    // "maximally fresh" (#2242, via the shared primitive).
    isTimestampNotFuture(f.generatedAt) &&
    Array.isArray(q.pools) &&
    // burn and choices are mandatory in QuotaView and dereferenced
    // unguarded by renderQuota (#2429): a feed without them is an old
    // collector's shape and is rejected whole, like other invalid
    // feeds, rather than crashing /quota.
    Array.isArray(q.burn) &&
    isQuotaChoices(q.choices) &&
    Array.isArray(f.attention) &&
    // chargesAttention is optional, but a present non-array value is
    // a shape failure, not data (#2439): the live source maps it
    // unguarded, so — like burn/choices above — the feed is rejected
    // whole. Malformed ELEMENTS inside an array are loadFeed's
    // drop-and-count concern, not the gate's.
    (f.chargesAttention === undefined || Array.isArray(f.chargesAttention)) &&
    (f.uninstrumentedServices === undefined || Array.isArray(f.uninstrumentedServices))
  );
}

export interface SubscriptionSanitization {
  /** Sanitized rows, or undefined when the section is absent (or was
   * dropped whole) — consumers already treat absence as an empty strip. */
  rows?: { name: string; plan: string; renewsAt: string | null }[];
  dropped: number;
}

/** One subscription row safe to hand to the renderers (#2446): the
 * strip maps esc(s.name)/esc(s.plan) and reads renewsAt unguarded, so
 * a null row or a non-string field 500s /quota and is served verbatim
 * by /api/quota.json. renewsAt must be null or a real ISO calendar
 * date (#3513): invalid strings survive sanitization otherwise and
 * render verbatim as renewal dates. */
export function isSubscriptionRow(v: unknown): v is { name: string; plan: string; renewsAt: string | null } {
  if (!isRecord(v)) return false;
  return typeof v.name === "string" && typeof v.plan === "string" && (v.renewsAt === null || isIsoDate(v.renewsAt));
}

/** Drop malformed subscription rows at the loadFeed boundary (#2446,
 * the #2429 pattern): the section is optional and unvalidated by
 * isFleetFeed, so element-level damage is dropped here rather than
 * rejecting the whole feed for one bad row. A non-array section is
 * dropped whole (treated as absent, counted as one drop). */
export function sanitizeSubscriptions(v: unknown): SubscriptionSanitization {
  if (v === undefined) return { dropped: 0 };
  if (!Array.isArray(v)) return { dropped: 1 };
  const rows = v.filter(isSubscriptionRow);
  return { rows, dropped: v.length - rows.length };
}

export interface UninstrumentedServiceSanitization {
  rows?: UninstrumentedSaasService[];
  dropped: number;
}

/** A tracker row safe to hand to renderers (#1172). */
export function isUninstrumentedSaasService(v: unknown): v is UninstrumentedSaasService {
  if (!isRecord(v)) return false;
  return (
    typeof v.name === "string" &&
    typeof v.whatFor === "string" &&
    typeof v.status === "string" &&
    (v.checkedAt === null || isIsoDate(v.checkedAt)) &&
    typeof v.overageBehavior === "string" &&
    typeof v.guardrailNotes === "string"
  );
}

/** Drop malformed tracker rows at the shared feed boundary. */
export function sanitizeUninstrumentedServices(v: unknown): UninstrumentedServiceSanitization {
  if (v === undefined) return { dropped: 0 };
  if (!Array.isArray(v)) return { dropped: 1 };
  const rows = v.filter(isUninstrumentedSaasService);
  return { rows, dropped: v.length - rows.length };
}

export interface RegistrySanitization {
  /** Safe registry rows, or undefined when the optional section is absent. */
  rows?: SessionRecord[];
  dropped: number;
}

/** Drop malformed optional registry rows at the shared loadFeed boundary
 * (#2539), so registry() and coverage() consume the same safe records.
 * An absent section stays absent; a non-array section becomes an empty
 * registry and counts as one dropped section; malformed array entries are
 * dropped individually. */
export function sanitizeRegistry(v: unknown): RegistrySanitization {
  if (v === undefined) return { dropped: 0 };
  // Shared container primitive (#2576 Phase 1): a non-array section is one
  // shape failure; malformed array entries drop individually and count.
  const parsed = arrayOf(isSessionRecord)(v);
  if (parsed === null) return { rows: [], dropped: 1 };
  return { rows: parsed.values, dropped: parsed.dropped };
}

/** A PAYG line safe for both the attention derivation and the quota renderer. */
export function isPaygLine(v: unknown): v is PaygLine {
  if (!isRecord(v)) return false;
  const nullableNonNegative = (value: unknown) => value === null || isNonNegative(value);
  const isEstimate = (value: unknown) => {
    if (value === null) return true;
    if (!isRecord(value)) return false;
    const { low, high } = value;
    return (
      isNonNegative(low) &&
      isNonNegative(high) &&
      low <= high &&
      typeof value.period === "string" &&
      (value.note === undefined || typeof value.note === "string")
    );
  };
  return (
    typeof v.id === "string" &&
    typeof v.label === "string" &&
    nullableNonNegative(v.accruedUsd) &&
    (v.collectedUsd === undefined || nullableNonNegative(v.collectedUsd)) &&
    (v.estimate === undefined || isEstimate(v.estimate)) &&
    nullableNonNegative(v.unbilledUsd) &&
    typeof v.note === "string" &&
    (v.measuredAt === null || isIsoDate(v.measuredAt)) &&
    (v.sourceClass === "api" ||
      v.sourceClass === "console" ||
      v.sourceClass === "manual" ||
      v.sourceClass === "estimate" ||
      v.sourceClass === "invoice")
  );
}

export interface PaygSanitization {
  /** Sanitized rows, or undefined when the optional section is absent/dropped whole. */
  rows?: PaygLine[];
  dropped: number;
}

/** Drop malformed PAYG rows at the shared feed boundary (#2450). */
export function sanitizePayg(v: unknown): PaygSanitization {
  if (v === undefined) return { dropped: 0 };
  if (!Array.isArray(v)) return { dropped: 1 };
  const rows = v.filter(isPaygLine);
  return { rows, dropped: v.length - rows.length };
}

export async function loadFeed(path: string): Promise<FleetFeed | null> {
  try {
    const parsed: unknown = JSON.parse(await readFile(path, "utf8"));
    if (!isFleetFeed(parsed)) return null;
    // The gate pins the section-level shape; element-level damage is
    // dropped here so every loadFeed consumer (live source, server)
    // sees only renderer-safe pools/windows (#2429). Drop counts are
    // attached to the feed so they are visible, not swallowed.
    const san = sanitizeQuotaView(parsed.quota);
    const meta = sanitizeFeedMetadata(parsed.meta);
    const saas = sanitizeSaasServices(parsed.saas);
    // Same drop-and-count treatment for chargesAttention elements
    // (#2439): the gate has already pinned the section to array-or-
    // absent, so arrayOf cannot return null here; malformed entries
    // drop individually and well-formed siblings still surface.
    const charges =
      parsed.chargesAttention === undefined ? undefined : arrayOf(isUnmatchedCharge)(parsed.chargesAttention);
    const subs = sanitizeSubscriptions(parsed.subscriptions);
    const uninstrumented = sanitizeUninstrumentedServices(parsed.uninstrumentedServices);
    // Registry rows feed both the session registry and coverage, so sanitize
    // once here rather than letting the two consumers see different inputs.
    const registry = sanitizeRegistry(parsed.registry);
    const payg = sanitizePayg(parsed.payg);
    // Charges rows get the same drop-and-count treatment at this same
    // shared gate (#2445): the section is optional, so absence stays
    // absent; a non-array section drops whole (counted as one dropped
    // section) instead of silently emptying the charges strip or
    // crashing a consumer; malformed rows drop individually. The
    // count rides out on the feed when drops occur, never swallowed.
    let chargesPart: { charges?: ChargeRow[]; chargesDropped?: number } = {};
    if (parsed.charges !== undefined) {
      const rows = arrayOf(isChargeRow)(parsed.charges);
      if (rows === null) {
        chargesPart = { charges: [], chargesDropped: 1 };
      } else {
        chargesPart = { charges: rows.values, ...(rows.dropped > 0 ? { chargesDropped: rows.dropped } : {}) };
      }
    }
    // The raw optional PAYG section may be malformed or absent. Remove
    // it (and any stale count) before projecting only the sanitized rows
    // and this load's own drop count.
    const feed = { ...parsed };
    delete feed.payg;
    delete feed.paygDropped;
    return {
      ...feed,
      meta,
      quota: san.view,
      saas: saas.rows,
      saasDropped: saas.dropped || undefined,
      quotaDropped: {
        pools: san.droppedPools,
        credentials: san.droppedCredentials,
        windows: san.droppedWindows,
        activeSessions: san.droppedActiveSessions,
        burn: san.droppedBurn,
        poolFreshness: san.droppedPoolFreshness,
        burnTokensPerMin: san.droppedBurnTokensPerMin,
        ...(san.droppedCredentialLifecycle > 0 ? { credentialLifecycle: san.droppedCredentialLifecycle } : {}),
      },
      ...(charges ? { chargesAttention: charges.values } : {}),
      ...(charges && charges.dropped > 0 ? { chargesAttentionDropped: charges.dropped } : {}),
      subscriptions: subs.rows,
      ...(subs.dropped > 0 ? { subscriptionsDropped: subs.dropped } : {}),
      ...(uninstrumented.rows ? { uninstrumentedServices: uninstrumented.rows } : {}),
      ...(uninstrumented.dropped > 0 ? { uninstrumentedServicesDropped: uninstrumented.dropped } : {}),
      ...(registry.rows !== undefined ? { registry: registry.rows } : {}),
      ...(registry.dropped > 0 ? { registryDropped: registry.dropped } : {}),
      ...(payg.rows !== undefined ? { payg: payg.rows } : {}),
      ...(payg.dropped > 0 ? { paygDropped: payg.dropped } : {}),
      ...chargesPart,
    };
  } catch {
    return null;
  }
}

/** "3m old", "2h old", "fresh" — for an age in milliseconds. */
export function ageMsText(ageMs: number): string {
  if (!Number.isFinite(ageMs)) return "unknown";
  const ms = Math.max(0, ageMs);
  if (ms < 90_000) return "fresh";
  if (ms < 3_600_000) return `${Math.round(ms / 60_000)}m old`;
  if (ms < 86_400_000) return `${(ms / 3_600_000).toFixed(1)}h old`;
  return `${Math.round(ms / 86_400_000)}d old`;
}

/** "3m old", "2h old", "fresh" — for provenance strings. */
export function feedAgeText(generatedAt: string, now = Date.now()): string {
  return ageMsText(now - Date.parse(generatedAt));
}
